use crate::app::models::AppToRun;
use os_api::{PriorityClass, OS};
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
    /// * [`AddAppsOutcome`] describing how many apps were added and the first parse error, if any
    ///
    /// # Note
    ///
    /// If the input vector is empty, the function returns an empty outcome without making any changes.
    /// If any application fails to parse, processing stops immediately and any already added
    /// applications remain in the group.
    pub fn add_app_to_group(&mut self, dropped_paths: Vec<std::path::PathBuf>) -> AddAppsOutcome {
        let mut outcome = AddAppsOutcome::default();

        if dropped_paths.is_empty() {
            return outcome;
        }

        for path in dropped_paths {
            let parsed_app_file = OS::parse_dropped_file(path.clone());

            match parsed_app_file {
                Ok((target, args)) => {
                    let app_to_run =
                        AppToRun::new(path, args, target, PriorityClass::Normal, false);

                    self.programs.push(app_to_run);
                    outcome.added_count += 1;
                }
                Err(err) => {
                    outcome.first_error = Some(err);
                    break;
                }
            }
        }

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::CoreGroup;

    #[cfg(target_os = "windows")]
    #[test]
    fn test_add_app_to_group_keeps_partial_success_before_first_error() {
        let mut group = CoreGroup {
            name: "Test".to_string(),
            cores: vec![0],
            programs: vec![],
            is_hidden: false,
            run_all_button: false,
        };

        let outcome = group.add_app_to_group(vec![
            r"C:\valid.exe".into(),
            r"C:\broken".into(),
            r"C:\later.exe".into(),
        ]);

        assert_eq!(outcome.added_count, 1);
        assert!(outcome
            .first_error
            .as_deref()
            .is_some_and(|message| message.contains("Failed to get file extension")));
        assert_eq!(group.programs.len(), 1);
        assert_eq!(
            group.programs[0].bin_path,
            std::path::PathBuf::from(r"C:\valid.exe")
        );
    }
}
