use os_api::PriorityClass;
use std::path::Path;

pub fn set_current_process_priority(priority: PriorityClass) -> Result<(), String> {
    os_api::OS::set_current_process_priority(priority)
}

pub fn get_cpu_model() -> String {
    os_api::OS::get_cpu_model()
}

pub fn supports_hide_to_tray() -> bool {
    os_api::OS::supports_hide_to_tray()
}

pub fn open_directory(path: &Path) -> Result<(), String> {
    os_api::OS::open_directory(path)
}

#[cfg(target_os = "windows")]
pub fn set_taskbar_visible(hwnd: windows::Win32::Foundation::HWND, visible: bool) {
    os_api::OS::set_taskbar_visible(hwnd, visible);
}

#[cfg(target_os = "windows")]
pub fn restore_and_focus_window(hwnd: windows::Win32::Foundation::HWND) {
    os_api::OS::restore_and_focus(hwnd);
}
