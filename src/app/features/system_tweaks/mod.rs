//! Saved profiles, one module registry and one application worker.
pub(crate) mod background_policy;
pub(crate) mod background_ui;
pub(crate) mod config;
pub(crate) mod execution_bridge;
pub(crate) mod pc_checks;
pub(crate) mod persistence;
pub(crate) mod platform;
pub(crate) mod platforms;
pub(crate) mod presentation;
pub(crate) mod service;
mod session_reducer;
pub(crate) mod tweaks;
use service::HeadlessTuning;
pub(crate) const MODULES_ENABLED: bool = cfg!(all(target_os = "windows", feature = "windows"));
pub(crate) use pc_checks::PcCheck;
pub(crate) use service::{SaveError, StatePersistence};
#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) use session_reducer::{RequestFence, TuningClient};
pub(crate) use session_reducer::{
    TuningConfigurationState, TuningOperationState, TuningRequest, TuningRuntime, TuningSnapshot,
    TweakCard,
};
pub(crate) struct Tuning {
    registry: platform::registry::Registry,
}
impl Default for Tuning {
    fn default() -> Self {
        Self::from_registry(platforms::registry())
    }
}
impl Tuning {
    pub(crate) fn registry(&self) -> &platform::registry::Registry {
        &self.registry
    }
    pub(crate) fn from_registry(registry: platform::registry::Registry) -> Self {
        Self { registry }
    }
    pub(crate) fn headless(
        &self,
        state: crate::app::models::AppStateStorage,
        persistence: Box<dyn StatePersistence>,
    ) -> HeadlessTuning {
        HeadlessTuning::new(state, persistence, self.registry.clone())
    }
    pub(crate) fn headless_selected(
        &self,
        state: crate::app::models::AppStateStorage,
    ) -> HeadlessTuning {
        self.headless(state, Box::new(service::SelectedStatePersistence))
    }
}
#[cfg(all(test, windows, feature = "tuning-test-controls"))]
mod native_roundtrip_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
pub(crate) mod simple_tests;
