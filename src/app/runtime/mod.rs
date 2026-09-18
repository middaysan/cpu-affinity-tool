mod state;

pub use state::{AppState, RunRuleOutcome};
#[cfg(test)]
pub(crate) use state::{CentralGroupSnapshot, CentralProgramSnapshot};
pub(crate) use state::{CentralPanelSnapshot, TrackedProcessSnapshot};
