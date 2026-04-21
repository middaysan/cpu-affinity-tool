use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::{GroupFormState, RunAppEditState};
use os_api::InstalledAppCatalogEntry;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[cfg(target_os = "windows")]
const TIPS: [&str; 5] = [
    "💡 Tip: Drag & drop executable files (.exe/.lnk) onto a group to add them, then click ▶ to run with the assigned CPU cores",
    "💡 Tip: Create different core groups for different types of applications to optimize performance",
    "💡 Tip: You can enable autorun for applications to start them automatically when the tool launches",
    "💡 Tip: Check the logs to see the history of application launches and their CPU affinity settings",
    "💡 Tip: Use the theme toggle button in the top-left corner to switch between light, dark, and system themes",
];

#[cfg(not(target_os = "windows"))]
const TIPS: [&str; 5] = [
    "💡 Tip: Drag & drop binaries or .desktop launchers onto a group to add them, then click ▶ to run with the assigned CPU cores",
    "💡 Tip: Create different core groups for different types of applications to optimize performance",
    "💡 Tip: You can enable autorun for applications to start them automatically when the tool launches",
    "💡 Tip: Check the logs to see the history of application launches and their CPU affinity settings",
    "💡 Tip: Use the theme toggle button in the top-left corner to switch between light, dark, and system themes",
];

#[derive(Default)]
pub(crate) struct InstalledAppPickerState {
    pub(crate) target_group_index: Option<usize>,
    pub(crate) query: String,
    pub(crate) entries: Vec<InstalledAppCatalogEntry>,
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) is_refreshing: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) needs_focus: bool,
    pub(crate) refresh_rx: Option<Receiver<Result<Vec<InstalledAppCatalogEntry>, String>>>,
}

/// Transient UI state owned by the runtime layer.
pub struct UiState {
    pub(crate) current_window: WindowRoute,
    pub(crate) group_form: GroupFormState,
    pub(crate) app_edit_state: RunAppEditState,
    pub(crate) dropped_files: Option<Vec<PathBuf>>,
    pub(crate) current_tip_index: usize,
    pub(crate) tip_change_interval: f64,
    pub(crate) last_tip_change_time: f64,
    pub(crate) installed_app_picker: InstalledAppPickerState,
}

impl UiState {
    pub fn new(total_threads: usize) -> Self {
        Self {
            current_window: WindowRoute::Groups(GroupRoute::List),
            group_form: GroupFormState {
                editing_index: None,
                editing_selection: None,
                core_selection: vec![false; total_threads],
                group_name: String::new(),
                run_all_enabled: false,
                last_clicked_core: None,
            },
            app_edit_state: RunAppEditState {
                current_edit: None,
                run_settings: None,
            },
            dropped_files: None,
            current_tip_index: 0,
            tip_change_interval: 120.0,
            last_tip_change_time: 0.0,
            installed_app_picker: InstalledAppPickerState::default(),
        }
    }

    pub fn reset_group_form(&mut self) {
        self.group_form.reset();
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        self.current_window = window;
    }

    pub fn current_tip(&mut self, current_time: f64) -> &str {
        let time_since_last_change = current_time - self.last_tip_change_time;
        if time_since_last_change >= self.tip_change_interval {
            self.current_tip_index = (self.current_tip_index + 1) % TIPS.len();
            self.last_tip_change_time = current_time;
        }

        TIPS[self.current_tip_index]
    }
}

#[cfg(test)]
mod tests {
    use super::UiState;
    use crate::app::navigation::{GroupRoute, WindowRoute};

    #[test]
    fn test_new_initializes_default_ui_state() {
        let state = UiState::new(6);
        assert!(matches!(
            state.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert_eq!(state.group_form.core_selection.len(), 6);
        assert!(state.app_edit_state.current_edit.is_none());
        assert!(state.dropped_files.is_none());
    }

    #[test]
    fn test_current_tip_rotates_only_after_interval() {
        let mut state = UiState::new(4);
        let first = state.current_tip(0.0).to_string();
        let still_first = state.current_tip(60.0).to_string();
        let second = state.current_tip(120.0).to_string();

        assert_eq!(first, still_first);
        assert_ne!(first, second);
    }

    #[test]
    fn test_new_initializes_installed_app_picker_closed() {
        let state = UiState::new(4);
        assert!(state.installed_app_picker.target_group_index.is_none());
        assert!(state.installed_app_picker.entries.is_empty());
        assert!(state.installed_app_picker.selected_entry_index.is_none());
    }
}
