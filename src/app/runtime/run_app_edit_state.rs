use crate::app::models::AppToRun;

/// State for editing an application that will be run with a specific CPU affinity.
pub struct RunAppEditState {
    /// The application currently being edited, if any
    pub current_edit: Option<AppToRun>,
    /// Optional run settings as a tuple of (group_index, program_index)
    pub run_settings: Option<(usize, usize)>,
}
