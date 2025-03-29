use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGroup {
    pub name: String,
    pub cores: Vec<usize>,
    pub programs: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
pub struct AppState {
    pub groups: Vec<CoreGroup>,
}
