mod migrations;
mod schema_refresh;
mod state_path;
mod storage_io;

#[cfg(test)]
mod tests;

use crate::app::features::rules::PersistedRuleIdentities;
use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_schema::CpuSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current version of the application state schema.
pub const CURRENT_APP_STATE_VERSION: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateStorageMode {
    LegacySidecar,
    PlatformData,
}

impl StateStorageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            StateStorageMode::LegacySidecar => "Legacy sidecar",
            StateStorageMode::PlatformData => "Platform data",
        }
    }
}

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
    /// Persisted logical identities for groups and rules in schema v6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_identities: Option<PersistedRuleIdentities>,
    #[serde(skip)]
    pub(crate) loaded_version: u32,
    #[serde(skip)]
    pub(crate) pending_pre_v6_backup: bool,
}

impl AppStateStorage {
    /// Loads the application state from the default JSON file.
    pub fn load_state() -> AppStateStorage {
        let path = state_path::get_state_path();
        Self::load_from_path(&path)
    }

    pub(crate) fn load_from_path(path: &Path) -> AppStateStorage {
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
    pub fn try_save_state(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let path = state_path::get_state_path();
        self.try_save_to_path(&path)
    }

    fn try_save_to_path(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if self.pending_pre_v6_backup {
            storage_io::backup_pre_v6_state_file(path)?;
        }
        self.save_to_path(path)?;
        self.loaded_version = self.version;
        self.pending_pre_v6_backup = false;
        Ok(())
    }

    pub fn active_data_dir() -> PathBuf {
        state_path::get_state_dir()
    }

    pub fn active_storage_mode() -> StateStorageMode {
        state_path::get_state_storage_mode()
    }

    pub fn mark_ready_for_v6_save(&mut self, rule_identities: PersistedRuleIdentities) {
        if self.loaded_version < CURRENT_APP_STATE_VERSION {
            self.pending_pre_v6_backup = true;
        }
        self.version = CURRENT_APP_STATE_VERSION;
        self.rule_identities = Some(rule_identities);
    }

    pub(crate) fn finalize_load(
        mut self,
        loaded_version: u32,
        pending_pre_v6_backup: bool,
    ) -> Self {
        self.loaded_version = loaded_version;
        self.pending_pre_v6_backup = pending_pre_v6_backup;
        self
    }
}
