mod migrations;
mod schema_refresh;
mod state_path;
mod storage_io;

#[cfg(test)]
mod tests;

use crate::app::features::rules::PersistedRuleIdentities;
use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_schema::CpuSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;
use std::path::{Path, PathBuf};
use std::{collections::BTreeMap, sync::Arc};

/// Current version of the application state schema.
pub const CURRENT_APP_STATE_VERSION: u32 = 11;

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
#[derive(Clone)]
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
    pub process_monitoring_enabled: bool,
    /// Start in the tray on Windows when a usable tray icon is available.
    pub start_minimized: bool,
    /// Enables the Windows-only, read-only Application Event Log diagnostic lookup.
    pub windows_event_log_diagnostics_enabled: bool,
    /// Persisted logical identities for groups and rules in schema v6.
    pub rule_identities: Option<PersistedRuleIdentities>,
    pub(crate) tuning: crate::app::features::system_tweaks::config::TuningEnvelope,
    pub(crate) loaded_version: u32,
    pub(crate) pending_pre_v6_backup: bool,
    extensions: BTreeMap<String, Arc<RawValue>>,
    save_admission: SaveAdmission,
}

#[derive(Clone, Default)]
enum SaveAdmission {
    #[default]
    Writable,

    Blocked {
        raw: Option<Arc<str>>,
        size: usize,
    },
}
#[derive(Deserialize)]
struct StateFields {
    version: u32,
    groups: Vec<CoreGroup>,
    cpu_schema: CpuSchema,
    theme_index: usize,
    #[serde(default)]
    process_monitoring_enabled: bool,
    #[serde(default)]
    start_minimized: bool,
    #[serde(default = "default_windows_event_log_diagnostics_enabled")]
    windows_event_log_diagnostics_enabled: bool,
    #[serde(default)]
    rule_identities: Option<PersistedRuleIdentities>,
    #[serde(default)]
    tuning: crate::app::features::system_tweaks::config::TuningEnvelope,
}
const STATE_FIELDS: &[&str] = &[
    "version",
    "groups",
    "cpu_schema",
    "theme_index",
    "process_monitoring_enabled",
    "start_minimized",
    "windows_event_log_diagnostics_enabled",
    "rule_identities",
    "tuning",
];

/// Borrow raw top-level values before typed decoding or legacy normalization.
/// Duplicate names are ambiguous evidence, including escaped spellings of a key.
struct RawStateFields<'a>(BTreeMap<String, &'a RawValue>);
impl<'de> Deserialize<'de> for RawStateFields<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = RawStateFields<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unambiguous state fields")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut fields = BTreeMap::new();
                while let Some((name, raw)) = map.next_entry::<String, &'de RawValue>()? {
                    if fields.insert(name, raw).is_some() {
                        return Err(serde::de::Error::custom("duplicate state field"));
                    }
                }
                Ok(RawStateFields(fields))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
impl<'de> Deserialize<'de> for AppStateStorage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let RawStateFields(mut all) =
            serde_json::from_str(raw.get()).map_err(serde::de::Error::custom)?;
        let fields: StateFields =
            serde_json::from_str(raw.get()).map_err(serde::de::Error::custom)?;
        for name in STATE_FIELDS {
            all.remove(*name);
        }
        Ok(Self {
            version: fields.version,
            groups: fields.groups,
            cpu_schema: fields.cpu_schema,
            theme_index: fields.theme_index,
            process_monitoring_enabled: fields.process_monitoring_enabled,
            start_minimized: fields.start_minimized,
            windows_event_log_diagnostics_enabled: fields.windows_event_log_diagnostics_enabled,
            rule_identities: fields.rule_identities,
            tuning: fields.tuning,
            loaded_version: 0,
            pending_pre_v6_backup: false,
            extensions: all
                .into_iter()
                .map(|(key, raw)| (key, Arc::from(raw.to_owned())))
                .collect(),
            save_admission: SaveAdmission::Writable,
        })
    }
}
impl Serialize for AppStateStorage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("version", &self.version)?;
        map.serialize_entry("groups", &self.groups)?;
        map.serialize_entry("cpu_schema", &self.cpu_schema)?;
        map.serialize_entry("theme_index", &self.theme_index)?;
        map.serialize_entry(
            "process_monitoring_enabled",
            &self.process_monitoring_enabled,
        )?;
        map.serialize_entry("start_minimized", &self.start_minimized)?;
        map.serialize_entry(
            "windows_event_log_diagnostics_enabled",
            &self.windows_event_log_diagnostics_enabled,
        )?;
        if let Some(identities) = &self.rule_identities {
            map.serialize_entry("rule_identities", identities)?;
        }
        if !self.tuning.is_off() {
            map.serialize_entry("tuning", &self.tuning)?;
        }
        for (name, raw) in &self.extensions {
            map.serialize_entry(name, raw.as_ref())?;
        }
        map.end()
    }
}

impl AppStateStorage {
    pub(crate) fn save_blocked(&self) -> bool {
        matches!(self.save_admission, SaveAdmission::Blocked { .. })
            || self.version > CURRENT_APP_STATE_VERSION
            || matches!(
                self.tuning.decode(),
                crate::app::features::system_tweaks::config::TuningLoad::Blocked
            )
    }
    #[cfg(test)]
    pub(crate) fn preserved_evidence(&self) -> Option<&str> {
        match &self.save_admission {
            SaveAdmission::Blocked { raw, .. } => raw.as_deref(),
            _ => None,
        }
    }
    fn block_save(&mut self, raw: Option<Arc<str>>, size: usize) {
        self.save_admission = SaveAdmission::Blocked { raw, size };
    }
    #[cfg(test)]
    pub(crate) fn test_fixture(configure: impl FnOnce(&mut Self)) -> Self {
        let mut state = schema_refresh::build_default_state();
        configure(&mut state);
        state
    }
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
        let read = storage_io::read_state_file_with_filesystem(path, filesystem);
        let data = match read {
            Ok(storage_io::StateRead::Text(data)) => Some(data),
            Ok(storage_io::StateRead::Oversized { size }) => {
                let mut blocked = schema_refresh::build_default_state();
                blocked.block_save(None, size);
                return blocked;
            }
            Err(_) => None,
        };
        if let Some(data) = &data {
            match migrations::version_selection(data) {
                migrations::VersionSelection::Blocked => {
                    let mut blocked = schema_refresh::build_default_state();
                    blocked.block_save(Some(Arc::from(data.as_str())), data.len());
                    return blocked;
                }
                migrations::VersionSelection::InvalidJson => {}
                selected => {
                    let raw_fields = match serde_json::from_str::<RawStateFields<'_>>(data) {
                        Ok(fields) => fields,
                        Err(_) => {
                            let mut blocked = schema_refresh::build_default_state();
                            blocked.block_save(Some(Arc::from(data.as_str())), data.len());
                            return blocked;
                        }
                    };
                    let tuning_present = raw_fields.0.contains_key("tuning");
                    let old_tuning = tuning_present
                        && !matches!(selected, migrations::VersionSelection::Known(11));
                    if let Some(mut state) = migrations::load_from_data(data, path) {
                        if old_tuning
                            || matches!(
                                state.tuning.decode(),
                                crate::app::features::system_tweaks::config::TuningLoad::Blocked
                            )
                        {
                            state.block_save(Some(Arc::from(data.as_str())), data.len());
                        }
                        return state;
                    }
                    if tuning_present {
                        // Invalid unrelated base fields cannot erase any tuning
                        // evidence merely because the full typed decode failed.
                        let mut blocked = schema_refresh::build_default_state();
                        blocked.block_save(Some(Arc::from(data.as_str())), data.len());
                        return blocked;
                    }
                }
            }
        }
        let mut default_state = schema_refresh::build_default_state();
        let recovered = (|| -> Result<(), Box<dyn std::error::Error>> {
            let serialized = storage_io::serialize_state_bounded(&default_state)?;
            storage_io::backup_state_file_with_filesystem(path, filesystem)?;
            storage_io::publish_state_bytes_with_filesystem(&serialized, path, filesystem)
        })();
        if let Err(error) = recovered {
            eprintln!("ERROR: Failed to recover unreadable state: {error}");
        }
        default_state.loaded_version = default_state.version;
        default_state
    }
    #[cfg_attr(test, allow(dead_code))]
    pub fn try_save_state(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let path = state_path::get_state_path();
        self.try_save_to_path(&path)
    }

    pub(crate) fn try_save_to_path(
        &mut self,
        path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut filesystem = storage_io::RealStateFilesystem;
        self.try_save_to_path_with_filesystem(path, &mut filesystem)
    }
    fn try_save_to_path_with_filesystem(
        &mut self,
        path: &Path,
        filesystem: &mut impl storage_io::StateFilesystem,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let SaveAdmission::Blocked { raw, size } = &self.save_admission {
            let refusal = if raw.is_some() {
                format!("state evidence is blocked ({size} bytes); save refused")
            } else {
                format!("state evidence exceeds read limit (at least {size} bytes); save refused")
            };
            return Err(refusal.into());
        }
        if self.save_blocked() {
            return Err("state evidence is blocked; save refused".into());
        }
        let serialized = storage_io::serialize_state_bounded(self)?;
        if self.pending_pre_v6_backup {
            storage_io::backup_pre_v6_state_file_with_filesystem(path, filesystem)?;
        }
        storage_io::publish_state_bytes_with_filesystem(&serialized, path, filesystem)?;
        self.loaded_version = self.version;
        self.pending_pre_v6_backup = false;
        Ok(())
    }

    pub fn active_data_dir() -> PathBuf {
        state_path::get_state_dir()
    }
    pub(crate) fn selected_state_path() -> PathBuf {
        state_path::get_state_path()
    }
    /// Independent recovery input: never repairs, migrates or replaces unreadable evidence.
    pub(crate) fn read_saved_for_tuning(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let filesystem = storage_io::RealStateFilesystem;
        let storage_io::StateRead::Text(raw) =
            storage_io::read_state_file_with_filesystem(path, &filesystem)?
        else {
            return Err("saved state exceeds read limit".into());
        };
        let mut state: Self = serde_json::from_str(&raw)?;
        if state.version != CURRENT_APP_STATE_VERSION || state.save_blocked() {
            return Err("saved state unavailable for tuning".into());
        }
        state.loaded_version = state.version;
        Ok(state)
    }
    pub(crate) fn publish_recovery_file(staging: &Path, destination: &Path) -> std::io::Result<()> {
        storage_io::publish_staged_file(staging, destination)?;
        storage_io::sync_parent_directory(destination)
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
