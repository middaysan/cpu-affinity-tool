use super::state_path::STATE_FILE_NAME;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub(super) fn read_state_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

pub(super) fn save_to_path<T: Serialize>(
    value: &T,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub(super) fn backup_state_file(path: &Path) {
    if !path.exists() {
        return;
    }

    let mut backup_path = PathBuf::from(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(STATE_FILE_NAME);

    let mut backup_name = format!("{file_name}.old");
    backup_path.set_file_name(&backup_name);

    let mut counter = 1;
    while backup_path.exists() {
        backup_name = format!("{file_name}.old{counter}");
        backup_path.set_file_name(&backup_name);
        counter += 1;
    }

    let _ = std::fs::rename(path, backup_path);
}
