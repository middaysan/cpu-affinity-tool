use crate::app::models::AppStateStorage;
use serde::Deserialize;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PreferencePatch {
    pub theme_index: Option<usize>,
    pub process_monitoring_enabled: Option<bool>,
    pub cpu_load_monitoring_enabled: Option<bool>,
    pub cpu_load_history_seconds: Option<u32>,
    pub start_minimized: Option<bool>,
    pub windows_event_log_diagnostics_enabled: Option<bool>,
}
impl PreferencePatch {
    pub fn apply(&self, state: &mut AppStateStorage) -> Result<(), String> {
        if self.theme_index.is_some_and(|theme| theme > 2)
            || self
                .cpu_load_history_seconds
                .is_some_and(|seconds| ![30, 300, 1800].contains(&seconds))
        {
            return Err("Unsupported theme or history duration".into());
        }
        macro_rules! update { ($($field:ident),*) => { $(if let Some(value) = self.$field { state.$field = value; })* }; }
        update!(
            theme_index,
            process_monitoring_enabled,
            cpu_load_monitoring_enabled,
            cpu_load_history_seconds,
            start_minimized,
            windows_event_log_diagnostics_enabled
        );
        Ok(())
    }
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
    use super::{toggle_process_monitoring, PreferencePatch};
    use crate::app::models::{AppStateStorage, CpuSchema};
    use std::sync::{Arc, RwLock};

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
            state.version = 5;
            state.groups = vec![];
            state.cpu_schema = CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            };
            state.theme_index = 0;
            state.process_monitoring_enabled = false;
            state.windows_event_log_diagnostics_enabled = true;
            state.start_minimized = false;
            state.rule_identities = None;
            state.loaded_version = 5;
            state.pending_pre_v6_backup = false;
            state.tuning = Default::default();
        })))
    }

    #[test]
    fn test_theme_selection_preserves_index_contract() {
        let state = sample_state();
        let mut state = state.write().unwrap();
        for theme in [1, 2, 0] {
            PreferencePatch {
                theme_index: Some(theme),
                ..Default::default()
            }
            .apply(&mut state)
            .unwrap();
            assert_eq!(state.theme_index, theme);
        }
        assert!(PreferencePatch {
            theme_index: Some(3),
            start_minimized: Some(true),
            ..Default::default()
        }
        .apply(&mut state)
        .is_err());
        assert_eq!(state.theme_index, 0);
        assert!(!state.start_minimized, "Invalid patches must be atomic");
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
