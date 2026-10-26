//! Read-only Windows CPU graphs. Independent of process settings enforcement.
mod api;
mod chart;
pub(crate) use api::CpuReadHandle;
mod contributors;
pub(crate) mod controls;
#[cfg(all(debug_assertions, feature = "debug-mcp"))]
pub(crate) mod debug_api;
mod export;
mod export_ui;
mod history;
mod memory;
mod meter;
mod mini_chart;
mod model;
mod process_list;
mod runtime;
pub(crate) mod view;
mod worker_history;
pub(crate) use model::RuleTarget;
pub(crate) use runtime::CpuLoadRuntime;
