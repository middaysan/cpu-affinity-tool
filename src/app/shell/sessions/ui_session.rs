use crate::app::shared::ids::GroupId;
use crate::app::shell::sessions::{GroupFormSession, InstalledAppPickerSession, RuleEditorSession};
use crate::app::shell::{GroupRoute, WindowRoute};
use std::path::PathBuf;

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

/// Transient UI state owned by the shell layer.
pub struct UiSession {
    pub current_window: WindowRoute,
    pub group_form: GroupFormSession,
    pub app_edit_state: RuleEditorSession,
    pub dropped_files: Option<Vec<PathBuf>>,
    pub file_drop_hover_target: Option<GroupId>,
    pub current_tip_index: usize,
    pub tip_change_interval: f64,
    pub last_tip_change_time: f64,
    pub installed_app_picker: InstalledAppPickerSession,
}

impl UiSession {
    pub fn new(total_threads: usize) -> Self {
        Self {
            current_window: WindowRoute::Groups(GroupRoute::List),
            group_form: GroupFormSession {
                editing_group_id: None,
                editing_selection: None,
                core_selection: vec![false; total_threads],
                group_name: String::new(),
                run_all_enabled: false,
                last_clicked_core: None,
            },
            app_edit_state: RuleEditorSession {
                current_edit: None,
                target: None,
                shortcut_result: None,
            },
            dropped_files: None,
            file_drop_hover_target: None,
            current_tip_index: 0,
            tip_change_interval: 120.0,
            last_tip_change_time: 0.0,
            installed_app_picker: InstalledAppPickerSession::default(),
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
    use super::UiSession;
    use crate::app::shell::{GroupRoute, WindowRoute};

    #[test]
    fn test_new_initializes_default_ui_state() {
        let state = UiSession::new(6);
        assert!(matches!(
            state.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert_eq!(state.group_form.core_selection.len(), 6);
        assert!(state.app_edit_state.current_edit.is_none());
        assert!(state.dropped_files.is_none());
        assert!(state.file_drop_hover_target.is_none());
    }

    #[test]
    fn test_current_tip_rotates_only_after_interval() {
        let mut state = UiSession::new(4);
        let first = state.current_tip(0.0).to_string();
        let still_first = state.current_tip(60.0).to_string();
        let second = state.current_tip(120.0).to_string();

        assert_eq!(first, still_first);
        assert_ne!(first, second);
    }

    #[test]
    fn test_new_initializes_installed_app_picker_closed() {
        let state = UiSession::new(4);
        assert!(state.installed_app_picker.target_group_id.is_none());
        assert!(state.installed_app_picker.entries.is_empty());
        assert!(state.installed_app_picker.selected_entry_index.is_none());
    }
}
