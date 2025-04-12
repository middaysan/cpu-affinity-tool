use serde::{Deserialize, Serialize};
use crate::app::os_cmd::{OsCmd, PriorityClass};
use crate::app::models::app_to_run::AppToRun;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGroup {
    pub name: String,
    pub cores: Vec<usize>,
    pub programs: Vec<AppToRun>,
    pub run_all_button: bool,
}

impl CoreGroup {
    pub fn add_app_to_group(&mut self, dropped_paths: Vec<std::path::PathBuf>) -> Result<(), String> {
        if dropped_paths.is_empty() {
            return Ok(());
        }

        for path in dropped_paths {
            let parsed_app_file = OsCmd::parse_dropped_file(path.clone());

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