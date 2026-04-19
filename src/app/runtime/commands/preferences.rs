use crate::app::models::AppStateStorage;
use std::sync::{Arc, RwLock};

pub fn toggle_theme(persistent_state: &Arc<RwLock<AppStateStorage>>) {
    {
        let mut state = persistent_state.write().unwrap();
        state.theme_index = (state.theme_index + 1) % 3;
    }
}

pub fn toggle_process_monitoring(persistent_state: &Arc<RwLock<AppStateStorage>>) {
    {
        let mut state = persistent_state.write().unwrap();
        state.process_monitoring_enabled = !state.process_monitoring_enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::{toggle_process_monitoring, toggle_theme};
    use crate::app::models::{AppStateStorage, CpuSchema};
    use std::sync::{Arc, RwLock};

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
            version: 4,
            groups: vec![],
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
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
}
