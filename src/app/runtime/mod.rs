mod state;

pub use state::AppState;
pub(crate) use state::CentralPanelSnapshot;
#[cfg(test)]
pub(crate) use state::{CentralGroupSnapshot, CentralProgramSnapshot};
