use std::path::PathBuf;

pub(super) const STATE_FILE_NAME: &str = "state.json";

pub(super) fn get_state_path() -> PathBuf {
    std::env::current_exe()
        .map(|mut path| {
            path.set_file_name(STATE_FILE_NAME);
            path
        })
        .unwrap_or_else(|_| STATE_FILE_NAME.into())
}
