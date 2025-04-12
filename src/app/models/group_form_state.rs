pub struct GroupFormState {
    pub editing_index: Option<usize>,
    pub editing_selection: Option<Vec<bool>>,
    pub core_selection: Vec<bool>,
    pub group_name: String,
    pub run_all_enabled: bool,
    pub is_visible: bool,
}

impl GroupFormState {
    /// Reset all group form fields to their default values.
    pub fn reset(&mut self) {
        self.editing_index = None;
        self.editing_selection = None;
        self.run_all_enabled = false;
        self.is_visible = false;
        self.group_name.clear();
        self.core_selection.fill(false);
    }
}
