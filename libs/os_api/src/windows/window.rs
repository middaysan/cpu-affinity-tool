use std::ptr::null_mut;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, EnumWindows, GWL_EXSTYLE, GetForegroundWindow,
    GetWindowLongW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_HIDE, SW_RESTORE,
    SW_SHOW, SetForegroundWindow, SetWindowLongW, ShowWindow, ShowWindowAsync, WM_NULL,
    WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use super::OS;

impl OS {
    pub fn focus_window_by_pid(pid: u32) -> bool {
        #[repr(C)]
        struct Ctx {
            target_pid: u32,
            found: HWND,
        }

        unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };

            let mut window_pid = 0u32;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            }

            if window_pid == ctx.target_pid && unsafe { IsWindowVisible(hwnd).as_bool() } {
                ctx.found = hwnd;
                return BOOL(0);
            }

            BOOL(1)
        }

        unsafe {
            let mut ctx = Box::new(Ctx {
                target_pid: pid,
                found: HWND(null_mut()),
            });

            let ctx_ptr = ctx.as_mut() as *mut Ctx;
            let _ = EnumWindows(Some(enum_windows_proc), LPARAM(ctx_ptr as isize));

            if ctx.found.0 == null_mut() {
                return false;
            }

            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let _ = ShowWindowAsync(ctx.found, SW_RESTORE);
            let _ = SetForegroundWindow(ctx.found);

            GetForegroundWindow() == ctx.found
        }
    }

    /// Toggles the window's visibility in the taskbar.
    /// show = true: Standard application window (AppWindow).
    /// show = false: Tool window (hidden from taskbar/Alt-Tab).
    pub fn set_taskbar_visible(hwnd: HWND, show: bool) {
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            let mut new_style = style;

            if show {
                new_style &= !(WS_EX_TOOLWINDOW.0 as i32);
                new_style |= WS_EX_APPWINDOW.0 as i32;
            } else {
                new_style |= WS_EX_TOOLWINDOW.0 as i32;
                new_style &= !(WS_EX_APPWINDOW.0 as i32);
            }

            if new_style != style {
                SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
            }
        }
    }

    pub fn hide_window(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    pub fn show_window(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }

    pub fn restore_and_focus(hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn poke_window(hwnd: HWND) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(None, WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
}
