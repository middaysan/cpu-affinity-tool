use super::{schema_refresh, AppStateStorage};
use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_schema::{CoreInfo, CoreType, CpuCluster, CpuSchema};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct VersionCheck {
    version: Option<u32>,
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
    let version_check: VersionCheck = serde_json::from_str(data).ok()?;

    match version_check.version {
        Some(7) => load_v7(data, path),
        Some(6) => load_v6(data, path),
        Some(5) => load_v5(data, path),
        Some(4) => load_v4(data, path),
        Some(3) => load_v3(data, path),
        Some(2) => load_v2(data, path),
        _ => load_legacy(data, path),
    }
}

fn load_v7(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    Some(state.finalize_load(7, false))
}

fn load_v6(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    state.backfill_tracked_process_names();
    Some(state.finalize_load(6, false))
}

fn load_v5(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    state.backfill_tracked_process_names();
    Some(state.finalize_load(5, true))
}

fn load_v4(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    let _ = schema_refresh::refresh_loaded_schema(&mut state);
    state.version = 5;
    state.rule_identities = None;
    state.backfill_tracked_process_names();
    Some(state.finalize_load(4, true))
}

fn load_v3(data: &str, _path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
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
        rule_identities: None,
        loaded_version: 0,
        pending_pre_v6_backup: false,
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
        rule_identities: None,
        loaded_version: 0,
        pending_pre_v6_backup: false,
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
