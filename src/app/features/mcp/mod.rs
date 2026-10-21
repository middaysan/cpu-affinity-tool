//! Optional local agent API. Domain behavior stays in the existing features.
mod actions;
mod activity;
mod activity_view;
mod catalog;
mod cpu_queries;
mod discovery;
mod operations;
mod policy;
mod reports;
mod runtime;
mod server;
mod services;
mod tweaks;
mod view;
mod wire_contracts;
pub(crate) use runtime::McpRuntime;

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[test]
    fn default_runtime_has_no_listener_or_credential_and_shutdown_is_idempotent() {
        let mut runtime = McpRuntime::default();
        assert!(!runtime.enabled());
        assert!(!runtime.listening());
        runtime.shutdown();
        runtime.shutdown();
        assert!(!runtime.listening());
    }
}

#[cfg(test)]
pub(crate) use server::assert_contract;
