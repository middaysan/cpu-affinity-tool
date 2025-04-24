use serde::{Deserialize, Serialize};
use os_api::{OS, PriorityClass};
use crate::app::models::app_to_run::AppToRun;

pub struct GroupFormState {
    pub editing_index: Option<usize>,
    pub editing_selection: Option<Vec<bool>>,
    pub core_selection: Vec<bool>,
    pub group_name: String,
    pub run_all_enabled: bool,
}

impl GroupFormState {
    /// Reset all group form fields to their default values.
    pub fn reset(&mut self) {
        self.editing_index = None;
        self.editing_selection = None;
        self.run_all_enabled = false;
        self.group_name.clear();
        self.core_selection.fill(false);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGroup {
    pub name: String,
    pub cores: Vec<usize>,
    pub programs: Vec<AppToRun>,
    pub is_hidden: bool,
    pub run_all_button: bool,
}

impl CoreGroup {
    pub fn add_app_to_group(&mut self, dropped_paths: Vec<std::path::PathBuf>) -> Result<(), String> {
        if dropped_paths.is_empty() {
            return Ok(());
        }

        for path in dropped_paths {
            let parsed_app_file = OS::parse_dropped_file(path.clone());

            match parsed_app_file {
                Ok((target, args)) => {
                    let app_to_run = AppToRun::new(
                        path, 
                        args, 
                        target,
                        PriorityClass::Normal,
                    );

                    self.programs.push(app_to_run);
                },
                Err(err) => return Err(err),
            }
        }

        Ok(())
    }
}