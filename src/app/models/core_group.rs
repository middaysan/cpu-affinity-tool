use crate::app::models::AppToRun;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddAppsOutcome {
    pub added_count: usize,
    pub first_error: Option<String>,
}

/// Represents a group of CPU cores and associated programs.
/// This structure is used to organize applications by the CPU cores they should run on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGroup {
    /// Display the name of the group
    pub name: String,
    /// Indices of the CPU cores that belong to this group
    pub cores: Vec<usize>,
    /// List of applications associated with this group
    pub programs: Vec<AppToRun>,
    /// Whether the group is hidden in the UI
    pub is_hidden: bool,
    /// Whether the "run all" button is enabled for this group
    pub run_all_button: bool,
}
