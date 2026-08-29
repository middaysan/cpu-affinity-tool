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
pub const CURRENT_APP_STATE_VERSION: u32 = 9;

pub(crate) const fn default_windows_event_log_diagnostics_enabled() -> bool {
    true
}

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
    /// Enables the Windows-only, read-only Application Event Log diagnostic lookup.
    #[serde(default = "default_windows_event_log_diagnostics_enabled")]
    pub windows_event_log_diagnostics_enabled: bool,
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
        let mut filesystem = storage_io::RealStateFilesystem;
        Self::load_from_path_with_filesystem(path, &mut filesystem)
    }

    pub(crate) fn load_from_path_with_filesystem(
        path: &Path,
        filesystem: &mut impl storage_io::StateFilesystem,
    ) -> AppStateStorage {
        storage_io::read_state_file_with_filesystem(path, filesystem)
            .and_then(|data| migrations::load_from_data(&data, path))
            .unwrap_or_else(|| {
                let default_state = schema_refresh::build_default_state();
                if let Err(error) = storage_io::backup_state_file_with_filesystem(path, filesystem)
                {
                    eprintln!(
                        "ERROR: Failed to preserve unreadable state before recovery: {error}"
                    );
                } else if let Err(error) =
                    storage_io::save_to_path_with_filesystem(&default_state, path, filesystem)
                {
                    eprintln!("ERROR: Failed to publish recovered default state: {error}");
                }

                default_state
            })
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn try_save_state(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let path = state_path::get_state_path();
        self.try_save_to_path(&path)
    }

    fn try_save_to_path(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut filesystem = storage_io::RealStateFilesystem;
        self.try_save_to_path_with_filesystem(path, &mut filesystem)
    }

    fn try_save_to_path_with_filesystem(
        &mut self,
        path: &Path,
        filesystem: &mut impl storage_io::StateFilesystem,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.pending_pre_v6_backup {
            storage_io::backup_pre_v6_state_file_with_filesystem(path, filesystem)?;
        }
        storage_io::save_to_path_with_filesystem(self, path, filesystem)?;
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

    pub fn mark_ready_for_current_schema_save(&mut self, rule_identities: PersistedRuleIdentities) {
        if self.loaded_version < 6 {
            self.pending_pre_v6_backup = true;
        }
        self.version = CURRENT_APP_STATE_VERSION;
        self.rule_identities = Some(rule_identities);
    }

    pub(crate) fn backfill_tracked_process_names(&mut self) -> bool {
        let mut changed = false;
        for group in &mut self.groups {
            for program in &mut group.programs {
                changed |= program.ensure_primary_process_name_tracked();
            }
        }
        changed
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
