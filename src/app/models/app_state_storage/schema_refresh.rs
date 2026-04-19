use super::{AppStateStorage, CURRENT_APP_STATE_VERSION};
use crate::app::models::cpu_presets::get_preset_for_model;
use crate::app::models::cpu_schema::{CoreType, CpuSchema};
use crate::app::models::meta::{effective_cpu_model, effective_total_threads, TEST_CPU_MODEL};

pub(super) fn build_default_state() -> AppStateStorage {
    let cpu_model = effective_cpu_model();
    let total_threads = effective_total_threads();
    let cpu_schema = get_preset_for_model(&cpu_model, total_threads).unwrap_or(CpuSchema {
        model: cpu_model,
        clusters: Vec::new(),
    });

    AppStateStorage {
        version: CURRENT_APP_STATE_VERSION,
        groups: Vec::new(),
        cpu_schema,
        theme_index: 0,
        process_monitoring_enabled: false,
    }
}

pub(super) fn refresh_loaded_schema(state: &mut AppStateStorage) -> bool {
    let cpu_model = effective_cpu_model();
    let total_threads = effective_total_threads();

    let is_generic = state.cpu_schema.model == "Generic CPU"
        || state.cpu_schema.clusters.is_empty()
        || state.cpu_schema.clusters.iter().all(|cluster| {
            cluster
                .cores
                .iter()
                .all(|core| core.core_type == CoreType::Other)
        });

    #[allow(clippy::const_is_empty)]
    if is_generic
        || !TEST_CPU_MODEL.is_empty()
        || (state.cpu_schema.model != cpu_model && !cpu_model.is_empty())
    {
        if let Some(preset) = get_preset_for_model(&cpu_model, total_threads) {
            state.cpu_schema = preset;
            return true;
        }

        if state.cpu_schema.model != cpu_model && !cpu_model.is_empty() {
            state.cpu_schema.model = cpu_model;
            return true;
        }
    }

    false
}

pub(super) fn refresh_migrated_schema(state: &mut AppStateStorage) {
    let cpu_model = effective_cpu_model();
    let total_threads = effective_total_threads();

    if let Some(preset) = get_preset_for_model(&cpu_model, total_threads) {
        state.cpu_schema = preset;
    } else if state.cpu_schema.clusters.is_empty() || state.cpu_schema.model == "Generic CPU" {
        state.cpu_schema.model = cpu_model;
    }
}
