use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub shortcut_path: PathBuf,
    pub target_path: PathBuf,
    pub arguments: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
    pub icon_index: i32,
}
