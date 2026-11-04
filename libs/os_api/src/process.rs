use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
    Realtime,
}

/// Result of a process-settings operation that verified the process instance first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessSettingsApplyOutcome {
    pub previous_affinity: usize,
    pub previous_priority: PriorityClass,
    pub affinity_changed: bool,
    pub priority_changed: bool,
}

/// Native settings failure classification, independent of localized Windows text.
#[derive(Clone, Debug)]
pub struct ProcessSettingsError {
    pub message: String,
    pub access_denied: bool,
}

impl std::fmt::Display for ProcessSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl From<String> for ProcessSettingsError {
    fn from(message: String) -> Self {
        Self {
            message,
            access_denied: false,
        }
    }
}

impl From<&str> for ProcessSettingsError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
