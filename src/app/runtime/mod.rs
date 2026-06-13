mod state;

pub(crate) use state::CentralPanelSnapshot;
pub use state::{AppState, RunRuleOutcome};
#[cfg(test)]
pub(crate) use state::{CentralGroupSnapshot, CentralProgramSnapshot};
