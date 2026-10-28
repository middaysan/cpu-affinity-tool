use std::ptr::null_mut;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, EnumThreadWindows, EnumWindows, GWL_EXSTYLE, GetClassNameW,
    GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, PostMessageW, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOZORDER, SetForegroundWindow, SetWindowLongW, SetWindowPos, ShowWindow, ShowWindowAsync,
    WM_NULL, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use windows::core::BOOL;

use super::OS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowActivation {
    Foreground,
    NotReady,
    Denied,
    Cancelled,
}

fn activation_preflight(
    owned: bool,
    visible: bool,
    minimized: bool,
    foreground: bool,
) -> Option<WindowActivation> {
    if !owned {
        Some(WindowActivation::Cancelled)
    } else if !visible || minimized {
        Some(WindowActivation::NotReady)
    } else if foreground {
        Some(WindowActivation::Foreground)
    } else {
        None
    }
}

#[test]
fn restored_owned_window_reaches_os_activation_after_pointer_motion() {
    assert_eq!(
        activation_preflight(true, true, false, false),
        None,
        "a raw last-input timestamp change must not veto an explicit Restore"
    );
    assert_eq!(
        activation_preflight(false, true, false, true),
        Some(WindowActivation::Cancelled)
    );
    assert_eq!(
        activation_preflight(true, false, false, true),
        Some(WindowActivation::NotReady)
    );
    assert_eq!(
        activation_preflight(true, true, true, true),
        Some(WindowActivation::NotReady)
    );
    assert_eq!(
        activation_preflight(true, true, false, true),
        Some(WindowActivation::Foreground)
    );
}

fn focus_window_rank(
    visible: bool,
    tool: bool,
    unowned: bool,
    width: i64,
    height: i64,
) -> Option<(bool, bool, i64)> {
    visible.then_some((!tool, unowned, width.max(0).saturating_mul(height.max(0))))
}
#[test]
fn s103_focus_prefers_a_main_window_over_small_helpers() {
    assert!(
        focus_window_rank(true, false, true, 1280, 720)
            > focus_window_rank(true, false, true, 100, 100)
    );
    assert!(
        focus_window_rank(true, false, true, 1280, 720)
            > focus_window_rank(true, false, false, 2000, 1000)
    );
    assert!(
        focus_window_rank(true, true, true, 100, 100).is_some(),
        "Tool-only ordinary applications retain a fallback"
    );
    assert!(
        focus_window_rank(true, false, false, 100, 100)
            > focus_window_rank(true, true, true, 1000, 1000)
    );
    assert_eq!(focus_window_rank(false, false, true, 100, 100), None);
}

impl OS {
    /// Activate only a restored window owned by this GUI thread. Never changes
    /// geometry, topmost state or input attachment. Windows owns foreground
    /// permission; a raw input timestamp also changes on harmless mouse motion.
    pub fn activate_own_window(hwnd: HWND) -> WindowActivation {
        use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
        unsafe {
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if let Some(outcome) = activation_preflight(
                thread == GetCurrentThreadId() && pid == GetCurrentProcessId(),
                IsWindowVisible(hwnd).as_bool(),
                IsIconic(hwnd).as_bool(),
                GetForegroundWindow() == hwnd,
            ) {
                return outcome;
            }
            let _ = SetForegroundWindow(hwnd);
            if GetForegroundWindow() == hwnd {
                WindowActivation::Foreground
            } else {
                WindowActivation::Denied
            }
        }
    }
}
fn qualified_foreground(window: usize, thread: u32, pid: u32) -> Result<(usize, u32), String> {
    if window == 0 || thread == 0 || pid == 0 {
        Err("foreground observation unavailable".into())
    } else {
        Ok((window, pid))
    }
}

impl OS {
    /// Keep Winit's event popup visible for WM_PAINT, but remove its minimized rectangle.
    /// Called on the GUI thread; windows on other threads/processes are never selected.
    pub fn normalize_winit_event_target_window() -> Result<(), String> {
        unsafe extern "system" fn select_helper(hwnd: HWND, lparam: LPARAM) -> BOOL {
            if is_winit_event_target_window(hwnd) {
                unsafe { *(lparam.0 as *mut Option<HWND>) = Some(hwnd) };
                return BOOL(0);
            }
            BOOL(1)
        }

        let mut helper = None;
        unsafe {
            let _ = EnumThreadWindows(
                windows::Win32::System::Threading::GetCurrentThreadId(),
                Some(select_helper),
                LPARAM((&mut helper as *mut Option<HWND>) as isize),
            );
        }
        let Some(helper) = helper else { return Ok(()) };
        unsafe {
            // Hiding this window breaks Winit's event delivery during interactive resizing.
            // Restore without activation, then remove the system's leftover minimized geometry.
            let _ = ShowWindow(helper, SW_SHOWNOACTIVATE);
            SetWindowPos(helper, None, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOZORDER)
                .map_err(|error| format!("Reset Winit event window geometry: {error}"))?;
            let mut rect = windows::Win32::Foundation::RECT::default();
            GetWindowRect(helper, &mut rect)
                .map_err(|error| format!("Verify Winit event window geometry: {error}"))?;
            let restored = !IsIconic(helper).as_bool();
            let visible = IsWindowVisible(helper).as_bool();
            let zero_sized = rect.right == rect.left && rect.bottom == rect.top;
            if !(restored && visible && zero_sized) {
                return Err(
                    "Winit event window did not return to its visible zero-sized state".into(),
                );
            }
        }
        Ok(())
    }

    pub fn foreground_process() -> Result<(usize, u32), String> {
        unsafe {
            let window = GetForegroundWindow();
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(window, Some(&mut pid));
            qualified_foreground(window.0 as usize, thread, pid)
        }
    }
    pub fn focus_window_by_pid(pid: u32) -> bool {
        #[repr(C)]
        struct Ctx {
            target_pid: u32,
            found: HWND,
            rank: Option<(bool, bool, i64)>,
        }

        unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };

            let mut window_pid = 0u32;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            }

            if window_pid == ctx.target_pid {
                use windows::Win32::UI::WindowsAndMessaging::{
                    GW_OWNER, GetWindow, GetWindowPlacement, WINDOWPLACEMENT,
                };
                let mut rect = windows::Win32::Foundation::RECT::default();
                let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
                if unsafe { IsIconic(hwnd).as_bool() } {
                    let mut placement = WINDOWPLACEMENT {
                        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                        ..Default::default()
                    };
                    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_ok() {
                        rect = placement.rcNormalPosition;
                    }
                }
                let rank = focus_window_rank(
                    unsafe { IsWindowVisible(hwnd).as_bool() },
                    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } & WS_EX_TOOLWINDOW.0 as i32 != 0,
                    unsafe { GetWindow(hwnd, GW_OWNER) }
                        .unwrap_or_default()
                        .0
                        .is_null(),
                    i64::from(rect.right) - i64::from(rect.left),
                    i64::from(rect.bottom) - i64::from(rect.top),
                );
                if rank > ctx.rank {
                    ctx.rank = rank;
                    ctx.found = hwnd;
                }
            }

            BOOL(1)
        }

        unsafe {
            let mut ctx = Box::new(Ctx {
                target_pid: pid,
                found: HWND(null_mut()),
                rank: None,
            });

            let ctx_ptr = ctx.as_mut() as *mut Ctx;
            let _ = EnumWindows(Some(enum_windows_proc), LPARAM(ctx_ptr as isize));

            if ctx.found.0.is_null() {
                return false;
            }

            let mut actual_pid = 0;
            GetWindowThreadProcessId(ctx.found, Some(&mut actual_pid));
            if actual_pid != pid {
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

fn is_winit_event_target_window(hwnd: HWND) -> bool {
    let expected_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
    let style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    if style & expected_style.0 != expected_style.0 {
        return false;
    }
    let mut class_name = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut class_name) };
    if length <= 0 {
        return false;
    }
    class_name[..length as usize]
        .iter()
        .copied()
        .eq("Winit Thread Event Target".encode_utf16())
}

#[cfg(test)]
mod p3_tests {
    use super::*;
    #[test]
    fn p3_foreground_null_invalid_and_zero_pid_are_unknown() {
        assert_eq!(qualified_foreground(1, 1, 42), Ok((1, 42)));
        assert!(qualified_foreground(0, 1, 42).is_err());
        assert!(qualified_foreground(1, 0, 42).is_err());
        assert!(qualified_foreground(1, 1, 0).is_err());
    }
}

#[cfg(test)]
mod startup_window_tests {
    use super::*;
    use windows::Win32::Foundation::{HINSTANCE, LRESULT, RECT};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_STYLE, GetWindowRect, IsIconic,
        RegisterClassW, SW_SHOWMINNOACTIVE, UnregisterClassW, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
    };
    use windows::core::{PCWSTR, w};

    struct TestWindow {
        hwnd: HWND,
        class: PCWSTR,
        instance: HINSTANCE,
    }

    impl TestWindow {
        fn new(class: PCWSTR) -> Self {
            unsafe extern "system" fn window_proc(
                hwnd: HWND,
                message: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> LRESULT {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }

            unsafe {
                let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
                assert_ne!(
                    RegisterClassW(&WNDCLASSW {
                        lpfnWndProc: Some(window_proc),
                        hInstance: instance,
                        lpszClassName: class,
                        ..Default::default()
                    }),
                    0
                );
                let mut window = Self {
                    hwnd: HWND::default(),
                    class,
                    instance,
                };
                window.hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance),
                    None,
                )
                .unwrap();
                window
            }
        }

        fn rect(&self) -> RECT {
            let mut rect = RECT::default();
            unsafe { GetWindowRect(self.hwnd, &mut rect).unwrap() };
            rect
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                if !self.hwnd.is_invalid() {
                    let _ = DestroyWindow(self.hwnd);
                }
                let _ = UnregisterClassW(self.class, Some(self.instance));
            }
        }
    }

    #[test]
    fn activation_waits_for_native_visibility_and_rejects_other_threads() {
        let window = TestWindow::new(w!("CPU Affinity Tool activation regression"));
        assert_eq!(
            OS::activate_own_window(window.hwnd),
            WindowActivation::NotReady
        );
        let raw = window.hwnd.0 as usize;
        std::thread::spawn(move || {
            assert_eq!(
                OS::activate_own_window(HWND(raw as *mut _)),
                WindowActivation::Cancelled
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    fn minimized_winit_helper_returns_to_zero_size_without_hiding_other_windows() {
        let helper = TestWindow::new(w!("Winit Thread Event Target"));
        let unrelated = TestWindow::new(w!("CPU Affinity Tool issue20 test window"));
        unsafe {
            let _ = ShowWindow(helper.hwnd, SW_SHOWMINNOACTIVE);
            let _ = ShowWindow(unrelated.hwnd, SW_SHOWMINNOACTIVE);
            // Winit installs this style after creation; the minimized rectangle can remain.
            SetWindowLongW(helper.hwnd, GWL_STYLE, (WS_VISIBLE | WS_POPUP).0 as i32);
        }
        let before = helper.rect();
        let original_style = unsafe { GetWindowLongW(helper.hwnd, GWL_EXSTYLE) };
        assert!(before.right > before.left && before.bottom > before.top);
        assert!(unsafe { IsIconic(unrelated.hwnd).as_bool() });

        OS::normalize_winit_event_target_window().unwrap();

        let after = helper.rect();
        assert_eq!(
            after.right - after.left,
            0,
            "leftover minimized helper rectangle"
        );
        assert_eq!(after.bottom - after.top, 0);
        assert!(!unsafe { IsIconic(helper.hwnd).as_bool() });
        assert!(
            unsafe { IsWindowVisible(helper.hwnd).as_bool() },
            "Winit needs WM_PAINT delivery"
        );
        assert!(
            unsafe { IsIconic(unrelated.hwnd).as_bool() },
            "unrelated windows must remain untouched"
        );
        assert_eq!(
            unsafe { GetWindowLongW(helper.hwnd, GWL_EXSTYLE) },
            original_style
        );
        OS::normalize_winit_event_target_window().unwrap();
        let repeated = helper.rect();
        assert_eq!(
            (
                repeated.right - repeated.left,
                repeated.bottom - repeated.top
            ),
            (0, 0)
        );
    }
}
