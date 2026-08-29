use crate::app::models::AppStateStorage;
use std::sync::{Arc, RwLock};

pub fn toggle_theme(persistent_state: &Arc<RwLock<AppStateStorage>>) {
    let mut state = persistent_state.write().unwrap();
    state.theme_index = (state.theme_index + 1) % 3;
}

pub fn toggle_process_monitoring(persistent_state: &Arc<RwLock<AppStateStorage>>) {
    let mut state = persistent_state.write().unwrap();
    state.process_monitoring_enabled = !state.process_monitoring_enabled;
}

#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
pub fn set_windows_event_log_diagnostics(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state = persistent_state
        .write()
        .map_err(|_| "could not update the diagnostics preference".to_string())?;
    state.windows_event_log_diagnostics_enabled = enabled;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "windows", feature = "windows"))]
    use super::set_windows_event_log_diagnostics;
    use super::{toggle_process_monitoring, toggle_theme};
    use crate::app::models::{AppStateStorage, CpuSchema};
    use std::sync::{Arc, RwLock};

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
            version: 5,
            groups: vec![],
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
            windows_event_log_diagnostics_enabled: true,
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }))
    }

    #[test]
    fn test_toggle_theme_cycles_theme_index() {
        let state = sample_state();
        toggle_theme(&state);
        assert_eq!(state.read().unwrap().theme_index, 1);
        toggle_theme(&state);
        toggle_theme(&state);
        assert_eq!(state.read().unwrap().theme_index, 0);
    }

    #[test]
    fn test_toggle_process_monitoring_flips_flag() {
        let state = sample_state();
        toggle_process_monitoring(&state);
        assert!(state.read().unwrap().process_monitoring_enabled);
    }

    #[cfg(any(target_os = "windows", feature = "windows"))]
    #[test]
    fn event_log_diagnostics_preference_is_idempotent() {
        let state = sample_state();

        set_windows_event_log_diagnostics(&state, false).unwrap();
        set_windows_event_log_diagnostics(&state, false).unwrap();

        let state = state.read().unwrap();
        assert!(!state.windows_event_log_diagnostics_enabled);
    }
}
