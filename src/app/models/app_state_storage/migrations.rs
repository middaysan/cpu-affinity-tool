use super::{schema_refresh, AppStateStorage};
use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_schema::{CoreInfo, CoreType, CpuCluster, CpuSchema};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

pub(super) enum VersionSelection {
    Known(u32),
    Legacy,
    InvalidJson,
    Blocked,
}
pub(super) fn version_selection(data: &str) -> VersionSelection {
    #[derive(Deserialize)]
    struct Header {
        #[serde(default)]
        version: VersionValue,
    }
    #[derive(Default)]
    enum VersionValue {
        #[default]
        Absent,
        Integer(u64),
        Malformed,
    }
    impl<'de> Deserialize<'de> for VersionValue {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = VersionValue;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("state version")
                }
                fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<VersionValue, E> {
                    Ok(VersionValue::Integer(n))
                }
                fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<VersionValue, E> {
                    Ok(VersionValue::Malformed)
                }
                fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<VersionValue, E> {
                    Ok(VersionValue::Malformed)
                }
                fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<VersionValue, E> {
                    Ok(VersionValue::Malformed)
                }
                fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<VersionValue, E> {
                    Ok(VersionValue::Malformed)
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<VersionValue, E> {
                    Ok(VersionValue::Malformed)
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut a: A,
                ) -> Result<VersionValue, A::Error> {
                    while a.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                    Ok(VersionValue::Malformed)
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut a: A,
                ) -> Result<VersionValue, A::Error> {
                    while a
                        .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                        .is_some()
                    {}
                    Ok(VersionValue::Malformed)
                }
            }
            d.deserialize_any(Visitor)
        }
    }
    match serde_json::from_str::<Header>(data) {
        Ok(Header {
            version: VersionValue::Absent,
        }) => VersionSelection::Legacy,
        Ok(Header {
            version: VersionValue::Integer(version @ 2..=11),
        }) => VersionSelection::Known(version as u32),
        Ok(_) => VersionSelection::Blocked,
        Err(_) if serde_json::from_str::<&serde_json::value::RawValue>(data).is_ok() => {
            // A valid JSON document with an ambiguous or unsupported header is
            // evidence to preserve, including duplicate explicit version fields.
            VersionSelection::Blocked
        }
        Err(_) => VersionSelection::InvalidJson,
    }
}

#[derive(Deserialize)]
struct V2AppStateStorage {
    _version: u32,
    groups: Vec<CoreGroup>,
    clusters: Vec<Vec<usize>>,
    theme_index: usize,
    process_monitoring_enabled: bool,
}

#[derive(Deserialize)]
struct LegacyAppStateStorage {
    groups: Vec<CoreGroup>,
    clusters: Vec<Vec<usize>>,
    theme_index: usize,
}

pub(super) fn load_from_data(data: &str, path: &Path) -> Option<AppStateStorage> {
    let selected = version_selection(data);
    let normalized_data;
    let data = match selected {
        VersionSelection::Known(10..=11) => data,
        _ => {
            normalized_data = with_legacy_descendant_default(data)?;
            normalized_data.as_str()
        }
    };

    match selected {
        VersionSelection::Known(11) => load_v11(data, path),
        VersionSelection::Known(10) => load_v10(data, path),
        VersionSelection::Known(9) => load_v9(data, path),
        VersionSelection::Known(8) => load_v8(data, path),
        VersionSelection::Known(7) => load_v7(data, path),
        VersionSelection::Known(6) => load_v6(data, path),
        VersionSelection::Known(5) => load_v5(data, path),
        VersionSelection::Known(4) => load_v4(data, path),
        VersionSelection::Known(3) => load_v3(data, path),
        VersionSelection::Known(2) => load_v2(data, path),
        VersionSelection::Legacy => load_legacy(data, path),
        _ => None,
    }
}

/// Schema versions before v10 managed descendants implicitly. Materialize that
/// historical default before deserializing rules so the rule type itself can use
/// the safer default for current-schema and schema-less inputs.
fn with_legacy_descendant_default(data: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(data).ok()?;
    let groups = value.get_mut("groups")?.as_array_mut()?;

    for group in groups {
        let Some(programs) = group.get_mut("programs").and_then(Value::as_array_mut) else {
            continue;
        };
        for program in programs {
            let Some(program) = program.as_object_mut() else {
                continue;
            };
            program
                .entry("manage_descendants")
                .or_insert(Value::Bool(true));
        }
    }

    serde_json::to_string(&value).ok()
}

fn load_v11(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    Some(state.finalize_load(11, false))
}
fn load_v10(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    Some(state.finalize_load(10, false))
}

fn load_v9(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    Some(state.finalize_load(9, false))
}

fn load_v8(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.windows_event_log_diagnostics_enabled = true;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    Some(state.finalize_load(8, false))
}

fn load_v7(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.windows_event_log_diagnostics_enabled = true;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    Some(state.finalize_load(7, false))
}

fn load_v6(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.windows_event_log_diagnostics_enabled = true;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    state.backfill_tracked_process_names();
    Some(state.finalize_load(6, false))
}

fn load_v5(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.windows_event_log_diagnostics_enabled = true;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    state.backfill_tracked_process_names();
    Some(state.finalize_load(5, true))
}

fn load_v4(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.windows_event_log_diagnostics_enabled = true;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    state.version = 5;
    state.rule_identities = None;
    state.backfill_tracked_process_names();
    Some(state.finalize_load(4, true))
}

fn load_v3(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.windows_event_log_diagnostics_enabled = true;
    state.version = 5;
    state.rule_identities = None;
    state.backfill_tracked_process_names();
    Some(state.finalize_load(3, true))
}

fn load_v2(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let v2: V2AppStateStorage = serde_json::from_str(data).ok()?;

    let mut migrated = AppStateStorage {
        version: 5,
        groups: v2.groups,
        cpu_schema: CpuSchema {
            model: "Generic CPU".to_string(),
            clusters: build_generic_clusters(v2.clusters),
        },
        theme_index: v2.theme_index,
        process_monitoring_enabled: v2.process_monitoring_enabled,
        cpu_load_monitoring_enabled: true,
        cpu_load_history_seconds: 300,
        windows_event_log_diagnostics_enabled: true,
        start_minimized: false,
        rule_identities: None,
        loaded_version: 0,
        pending_pre_v6_backup: false,
        tuning: Default::default(),
        extensions: Default::default(),
        save_admission: Default::default(),
    };

    schema_refresh::refresh_migrated_schema(&mut migrated);
    migrated.backfill_tracked_process_names();
    Some(migrated.finalize_load(2, true))
}

fn load_legacy(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let legacy: LegacyAppStateStorage = serde_json::from_str(data).ok()?;

    let mut migrated = AppStateStorage {
        version: 5,
        groups: legacy.groups,
        cpu_schema: CpuSchema {
            model: "Generic CPU".to_string(),
            clusters: build_generic_clusters(legacy.clusters),
        },
        theme_index: legacy.theme_index,
        process_monitoring_enabled: false,
        cpu_load_monitoring_enabled: true,
        cpu_load_history_seconds: 300,
        windows_event_log_diagnostics_enabled: true,
        start_minimized: false,
        rule_identities: None,
        loaded_version: 0,
        pending_pre_v6_backup: false,
        tuning: Default::default(),
        extensions: Default::default(),
        save_admission: Default::default(),
    };

    schema_refresh::refresh_migrated_schema(&mut migrated);
    migrated.backfill_tracked_process_names();
    Some(migrated.finalize_load(0, true))
}

pub(super) fn build_generic_clusters(clusters: Vec<Vec<usize>>) -> Vec<CpuCluster> {
    clusters
        .into_iter()
        .enumerate()
        .map(|(index, cluster_cores)| CpuCluster {
            name: format!("Cluster {}", index + 1),
            cores: cluster_cores
                .into_iter()
                .map(|core_index| CoreInfo {
                    index: core_index,
                    core_type: CoreType::Other,
                    label: format!("Core {core_index}"),
                })
                .collect(),
        })
        .collect()
}
