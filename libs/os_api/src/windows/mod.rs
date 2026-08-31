mod common;
mod cpu;
mod event_log;
mod ipc;
mod launch;
mod processes;
mod scheduling;
mod shell;
mod window;

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
