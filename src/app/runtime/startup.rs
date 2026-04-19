use super::state::AppState;
use crate::app::models::{effective_cpu_model, effective_total_threads};
use regex::Regex;

pub fn log_startup(state: &mut AppState) {
    state
        .log_manager
        .add_sticky_once("Application started".into());

    let model = effective_cpu_model();
    let threads = effective_total_threads();
    state
        .log_manager
        .add_sticky_once(format!("Detected CPU: \"{}\" ({} threads)", model, threads));

    let presets_info = crate::app::models::cpu_presets::get_all_presets_info();
    state.log_manager.add_sticky_once(format!(
        "Loaded {} CPU presets from embedded JSON",
        presets_info.len()
    ));

    {
        let storage = state.persistent_state.read().unwrap();
        if storage.cpu_schema.clusters.is_empty() {
            state
                .log_manager
                .add_sticky_once("CPU layout: Generic (no clusters)".into());

            for (name, regexes, preset_threads) in presets_info {
                let regex_match = if regexes.is_empty() {
                    false
                } else {
                    regexes.iter().any(|pattern| {
                        Regex::new(pattern)
                            .map(|regex| regex.is_match(&model))
                            .unwrap_or(false)
                    })
                };

                if regex_match {
                    if let Some(expected_threads) = preset_threads {
                        if expected_threads != threads {
                            state.log_manager.add_sticky_once(format!(
                                "Note: Preset \"{}\" matches regex but expects {} threads (you have {})",
                                name, expected_threads, threads
                            ));
                        }
                    }
                }
            }
        } else {
            state.log_manager.add_sticky_once(format!(
                "CPU layout: {} ({} clusters)",
                storage.cpu_schema.model,
                storage.cpu_schema.clusters.len()
            ));
        }
    }
}
