use super::{schema_refresh, storage_io, AppStateStorage, CURRENT_APP_STATE_VERSION};
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
        Some(4) => load_v4(data, path),
        Some(3) => load_v3(data, path),
        Some(2) => load_v2(data, path),
        _ => load_legacy(data, path),
    }
}

fn load_v4(data: &str, path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;

    if schema_refresh::refresh_loaded_schema(&mut state) {
        let _ = state.save_to_path(path);
    }

    Some(state)
}

fn load_v3(data: &str, path: &Path) -> Option<AppStateStorage> {
    let mut state: AppStateStorage = serde_json::from_str(data).ok()?;
    state.version = CURRENT_APP_STATE_VERSION;
    let _ = state.save_to_path(path);
    Some(state)
}

fn load_v2(data: &str, path: &Path) -> Option<AppStateStorage> {
    let v2: V2AppStateStorage = serde_json::from_str(data).ok()?;

    let mut migrated = AppStateStorage {
        version: CURRENT_APP_STATE_VERSION,
        groups: v2.groups,
        cpu_schema: CpuSchema {
            model: "Generic CPU".to_string(),
            clusters: build_generic_clusters(v2.clusters),
        },
        theme_index: v2.theme_index,
        process_monitoring_enabled: v2.process_monitoring_enabled,
    };

    schema_refresh::refresh_migrated_schema(&mut migrated);
    storage_io::backup_state_file(path);
    let _ = migrated.save_to_path(path);

    Some(migrated)
}

fn load_legacy(data: &str, path: &Path) -> Option<AppStateStorage> {
    let legacy: LegacyAppStateStorage = serde_json::from_str(data).ok()?;

    let mut migrated = AppStateStorage {
        version: CURRENT_APP_STATE_VERSION,
        groups: legacy.groups,
        cpu_schema: CpuSchema {
            model: "Generic CPU".to_string(),
            clusters: build_generic_clusters(legacy.clusters),
        },
        theme_index: legacy.theme_index,
        process_monitoring_enabled: false,
    };

    schema_refresh::refresh_migrated_schema(&mut migrated);
    storage_io::backup_state_file(path);
    let _ = migrated.save_to_path(path);

    Some(migrated)
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
