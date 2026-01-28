use crate::app::models::app_to_run::AppToRun;
use os_api::{PriorityClass, OS};
use serde::{Deserialize, Serialize};

/// Represents the state of the form used for creating or editing a core group.
/// This structure tracks the form's input fields and editing state.
pub struct GroupFormState {
    /// Index of the group being edited, or None if creating a new group
    pub editing_index: Option<usize>,
    /// Current selection state when editing an existing group, or None if creating a new group
    pub editing_selection: Option<Vec<bool>>,
    /// Boolean vector representing which CPU cores are selected in the form
    pub core_selection: Vec<bool>,
    /// Name of the group being created or edited
    pub group_name: String,
    /// Whether the "run all" button should be enabled for this group
    pub run_all_enabled: bool,
    /// Index of the last clicked core for shift+click range selection
    pub last_clicked_core: Option<usize>,
}

impl GroupFormState {
    /// Reset all group form fields to their default values.
    ///
    /// Clears the editing state, disables the "run all" button,
    /// clears the group name, and deselects all cores.
    pub fn reset(&mut self) {
        self.editing_index = None;
        self.editing_selection = None;
        self.run_all_enabled = false;
        self.group_name.clear();
        self.core_selection.fill(false);
        self.last_clicked_core = None;
    }
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

impl CoreGroup {
    /// Adds applications to the group from a list of file paths.
    ///
    /// For each path in the list, attempts to parse it as an application file
    /// using the OS-specific parser. If successful, creates a new `AppToRun` instance
    /// with default settings (Normal priority, autorun disabled) and adds it to the group.
    ///
    /// # Parameters
    ///
    /// * `dropped_paths` - A vector of paths to application files
    ///
    /// # Returns
    ///
    /// * `Ok(())` if all applications were added successfully
    /// * `Err(String)` with an error message if any application failed to parse
    ///
    /// # Note
    ///
    /// If the input vector is empty, the function returns `Ok(())` without making any changes.
    /// If any application fails to parse, the function returns immediately with the error,
    /// and any applications that were already added remain in the group.
    pub fn add_app_to_group(
        &mut self,
        dropped_paths: Vec<std::path::PathBuf>,
    ) -> Result<(), String> {
        if dropped_paths.is_empty() {
            return Ok(());
        }

        for path in dropped_paths {
            let parsed_app_file = OS::parse_dropped_file(path.clone());

            match parsed_app_file {
                Ok((target, args)) => {
                    let app_to_run =
                        AppToRun::new(path, args, target, PriorityClass::Normal, false);

                    self.programs.push(app_to_run);
                }
                Err(err) => return Err(err),
            }
        }

        Ok(())
    }
}
