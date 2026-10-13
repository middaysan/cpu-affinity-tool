mod common;
mod cpu;
mod event_log;
pub(crate) mod gaming;
mod gaming_diagnostics;
mod ipc;
mod launch;
pub(crate) mod pc_checks;
mod processes;
mod scheduling;
mod shell;
pub(crate) mod tuning;
mod window;
pub(crate) mod windows_tweaks;

pub use event_log::WindowsApplicationFailure;

pub use ipc::{
    LocalIpcClientError, LocalIpcEndpoint, LocalIpcGuard, LocalIpcRequest, LocalIpcServer,
    LocalIpcWake,
};

pub struct OS;

impl OS {
    pub const fn supports_hide_to_tray() -> bool {
        true
    }

    pub const fn supports_installed_app_picker() -> bool {
        true
    }
}
