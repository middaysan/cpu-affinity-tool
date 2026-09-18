use crate::app::models::AppStateStorage;
use std::sync::{Arc, RwLock};

pub fn set_theme_index(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    theme_index: usize,
) -> bool {
    let mut state = persistent_state.write().unwrap();
    if theme_index > 2 || state.theme_index == theme_index {
        return false;
    }
    state.theme_index = theme_index;
    true
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
    use super::{set_theme_index, toggle_process_monitoring};
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
            start_minimized: false,
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }))
    }

    #[test]
    fn test_theme_selection_preserves_index_contract() {
        let state = sample_state();
        assert!(set_theme_index(&state, 1));
        assert_eq!(state.read().unwrap().theme_index, 1);
        assert!(set_theme_index(&state, 2));
        assert_eq!(state.read().unwrap().theme_index, 2);
        assert!(set_theme_index(&state, 0));
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
