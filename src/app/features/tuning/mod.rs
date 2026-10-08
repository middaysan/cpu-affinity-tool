// Saved transactions and immutable snapshots are composed into AppState. The
// shipping catalog remains empty and no native Game/logon/boot producer exists.

pub(crate) mod config;
pub(crate) mod configured_source;
pub(crate) mod execution_bridge;
pub mod platform;
pub(crate) mod presentation;
pub(crate) mod service;
mod session_reducer;
pub(crate) use service::{HeadlessTuning, SaveError, StatePersistence};
#[cfg(test)]
pub(crate) use session_reducer::ModuleCard;
pub(crate) use session_reducer::{
    TuningConfigurationState, TuningOperationState, TuningRequest, TuningRuntime, TuningSnapshot,
};

/// Public feature facade. Concrete shipping registrations belong only in this
/// composition function; the staged foundation registers no fake or OS module.
#[derive(Default)]
pub struct Tuning {
    catalog: platform::catalog::Catalog,
}

impl Tuning {
    pub fn catalog(&self) -> &platform::catalog::Catalog {
        &self.catalog
    }

    pub(crate) fn headless(
        &self,
        state: crate::app::models::AppStateStorage,
        persistence: Box<dyn StatePersistence>,
    ) -> HeadlessTuning {
        HeadlessTuning::new(state, persistence, self.catalog().clone())
    }
    pub(crate) fn headless_selected(
        &self,
        state: crate::app::models::AppStateStorage,
    ) -> HeadlessTuning {
        self.headless(state, Box::new(service::SelectedStatePersistence))
    }
}

#[cfg(test)]
mod testing;
