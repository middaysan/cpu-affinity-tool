use crate::app::shared::ids::GroupId;
use crate::app::shell::sessions::{GroupFormSession, InstalledAppPickerSession, RuleEditorSession};
use crate::app::shell::{GroupRoute, WindowRoute};
use std::path::PathBuf;

/// Transient UI state owned by the shell layer.
pub struct UiSession {
    pub current_window: WindowRoute,
    pub group_form: GroupFormSession,
    pub app_edit_state: RuleEditorSession,
    pub dropped_files: Option<Vec<PathBuf>>,
    pub file_drop_hover_target: Option<GroupId>,
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
            installed_app_picker: InstalledAppPickerSession::default(),
        }
    }

    pub fn reset_group_form(&mut self) {
        self.group_form.reset();
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        self.current_window = window;
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
    fn test_new_initializes_installed_app_picker_closed() {
        let state = UiSession::new(4);
        assert!(state.installed_app_picker.target_group_id.is_none());
        assert!(state.installed_app_picker.entries.is_empty());
        assert!(state.installed_app_picker.selected_entry_index.is_none());
    }
}
