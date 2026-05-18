use crate::app::shared::ids::GroupId;

/// Represents the state of the form used for creating or editing a core group.
/// This structure tracks the form's input fields and editing state.
pub struct GroupFormSession {
    /// Transient logical identity of the group being edited.
    pub editing_group_id: Option<GroupId>,
    /// Current selection state when editing an existing group.
    pub editing_selection: Option<Vec<bool>>,
    /// Boolean vector representing which CPU cores are selected in the form.
    pub core_selection: Vec<bool>,
    /// Name of the group being created or edited.
    pub group_name: String,
    /// Whether the "run all" button should be enabled for this group.
    pub run_all_enabled: bool,
    /// Index of the last clicked core for shift+click range selection.
    pub last_clicked_core: Option<usize>,
}

impl GroupFormSession {
    /// Reset all group form fields to their default values.
    pub fn reset(&mut self) {
        self.editing_group_id = None;
        self.editing_selection = None;
        self.run_all_enabled = false;
        self.group_name.clear();
        self.core_selection.fill(false);
        self.last_clicked_core = None;
    }
}
