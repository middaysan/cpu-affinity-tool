#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) mod cpu_load;
pub mod diagnostics;
pub mod execution;
#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) mod mcp;
pub mod preferences;
pub mod rules;
pub mod shortcut;
pub mod system_tweaks;
pub mod topology;
