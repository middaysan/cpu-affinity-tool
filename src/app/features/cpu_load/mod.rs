//! Read-only Windows CPU graphs. Independent of process settings enforcement.
mod chart;
mod contributors;
#[cfg(all(debug_assertions, feature = "debug-mcp"))]
pub(crate) mod debug_api;
mod history;
mod meter;
mod mini_chart;
mod model;
mod process_list;
mod runtime;
pub(crate) mod view;
mod worker_history;
pub(crate) use model::RuleTarget;
pub(crate) use runtime::CpuLoadRuntime;
