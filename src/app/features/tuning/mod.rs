// This foundation is intentionally dormant until the F4 application integration.
#![allow(dead_code)]

pub(crate) mod config;
pub(crate) mod configured_source;
pub mod platform;
mod service;
pub(crate) use service::{HeadlessTuning, StatePersistence};

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
        HeadlessTuning::new(state, persistence, self.catalog.clone())
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
