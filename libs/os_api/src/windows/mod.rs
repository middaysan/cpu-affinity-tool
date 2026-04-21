mod common;
mod cpu;
mod launch;
mod processes;
mod scheduling;
mod shell;
mod window;

pub struct OS;

impl OS {
    pub const fn supports_hide_to_tray() -> bool {
        true
    }

    pub const fn supports_installed_app_picker() -> bool {
        true
    }
}
