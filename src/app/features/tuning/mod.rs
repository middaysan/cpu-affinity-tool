// This foundation is intentionally dormant until the F4 application integration.
#![allow(dead_code)]

pub(crate) mod config;
pub(crate) mod configured_source;
pub mod platform;

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
}

#[cfg(test)]
mod testing;
