use crate::app::models::app_to_run;

pub struct RunAppEditState {
    pub current_edit: Option<app_to_run::AppToRun>,
    pub run_settings: Option<(usize, usize)>,
}
