use crate::app::models::{
    effective_cpu_model, effective_total_threads, AppStateStorage, LogManager,
};
use regex::Regex;
use std::sync::{Arc, RwLock};

#[cfg(all(target_os = "windows", feature = "render-profiling"))]
pub(crate) mod render_profile;

#[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
pub(crate) mod debug_mcp;

#[cfg(all(not(debug_assertions), feature = "debug-mcp"))]
compile_error!("debug-mcp is forbidden in release builds; use the debug profile");

#[macro_export]
macro_rules! debug_mcp_control {
    ($id:expr, $response:expr) => {
        #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
        $crate::app::features::diagnostics::debug_mcp::control($id, &$response);
        #[cfg(not(all(target_os = "windows", debug_assertions, feature = "debug-mcp")))]
        let _ = &$response;
    };
}

/// Entirely absent from ordinary builds, including the clock reads.
#[macro_export]
macro_rules! render_profile_scope {
    ($stage:ident) => {
        #[cfg(all(target_os = "windows", feature = "render-profiling"))]
        let _render_profile_scope = $crate::app::features::diagnostics::render_profile::span(
            $crate::app::features::diagnostics::render_profile::Stage::$stage,
        );
    };
}

#[cfg(all(test, not(all(target_os = "windows", feature = "windows"))))]
#[allow(dead_code)]
pub mod crash_reports;
#[cfg(all(target_os = "windows", feature = "windows"))]
pub mod crash_reports;
#[cfg(all(target_os = "windows", feature = "windows"))]
pub mod windows_event_log;

pub type DiagnosticEvent = crate::app::shell::events::ShellEvent;

pub fn log_startup(log_manager: &mut LogManager, persistent_state: &Arc<RwLock<AppStateStorage>>) {
    log_manager.add_sticky_once("Application started".into());

    let model = effective_cpu_model();
    let threads = effective_total_threads();
    log_manager.add_sticky_once(format!("Detected CPU: \"{}\" ({} threads)", model, threads));

    let presets_info = crate::app::models::cpu_presets::get_all_presets_info();
    log_manager.add_sticky_once(format!(
        "Loaded {} CPU presets from embedded JSON",
        presets_info.len()
    ));

    let storage = persistent_state.read().unwrap();
    if storage.cpu_schema.clusters.is_empty() {
        log_manager.add_sticky_once("CPU layout: Generic (no clusters)".into());

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
                        log_manager.add_sticky_once(format!(
                            "Note: Preset \"{}\" matches regex but expects {} threads (you have {})",
                            name, expected_threads, threads
                        ));
                    }
                }
            }
        }
    } else {
        log_manager.add_sticky_once(format!(
            "CPU layout: {} ({} clusters)",
            storage.cpu_schema.model,
            storage.cpu_schema.clusters.len()
        ));
    }
}
