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
