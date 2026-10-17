use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Globalization::{CP_ACP, MB_PRECOMPOSED, MultiByteToWideChar};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Shell::{DROPFILES, DragAcceptFiles, DragFinish, DragQueryPoint, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CHANGEFILTERSTRUCT, CallWindowProcW, ChangeWindowMessageFilterEx, DefWindowProcW, GWL_EXSTYLE,
    GWLP_WNDPROC, GetPropW, GetWindowLongPtrW, GetWindowLongW, GetWindowThreadProcessId,
    MSGFLT_ALLOW, MSGFLT_RESET, MSGFLTINFO_NONE, RemovePropW, SetPropW, SetWindowLongPtrW,
    WM_COPYDATA, WM_DROPFILES, WM_NCDESTROY, WNDPROC, WS_EX_ACCEPTFILES,
};

const DROP_PROPERTY: windows::core::PCWSTR = windows::core::w!("CpuAffinityTool.ShellFileDrop");
// Shell uses this system message to marshal an HDROP across processes. GLFW uses
// the same window-scoped exception; it is not an application command channel.
const WM_COPYGLOBALDATA: u32 = 0x0049;
const FILE_MESSAGES: [u32; 3] = [WM_DROPFILES, WM_COPYDATA, WM_COPYGLOBALDATA];
const MAX_DROP_BYTES: usize = 1024 * 1024;
const MAX_DROP_FILES: usize = 128;
const MAX_PATH_UNITS: usize = 32767;

#[derive(Debug, PartialEq, Eq)]
pub struct WindowFileDrop {
    pub paths: Vec<PathBuf>,
    /// Physical client coordinates captured by Shell at release, not at GUI polling.
    pub position: [i32; 2],
}

struct DropSink {
    sender: SyncSender<WindowFileDrop>,
    wake: Arc<dyn Fn() + Send + Sync>,
    previous: WNDPROC,
}

/// GUI-thread registration. Rc prevents moving subclass ownership to another thread.
pub struct WindowFileDropGuard {
    hwnd: HWND,
    sink: Rc<DropSink>,
    receiver: Receiver<WindowFileDrop>,
    filters_changed: [bool; 3],
    accepted_before: bool,
}

impl WindowFileDropGuard {
    pub fn install(window: usize, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let hwnd = HWND(window as *mut _);
        let mut pid = 0;
        let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if thread != unsafe { GetCurrentThreadId() } || pid != unsafe { GetCurrentProcessId() } {
            return Err("File-drop window must belong to the current GUI thread".into());
        }
        if !unsafe { GetPropW(hwnd, DROP_PROPERTY) }.is_invalid() {
            return Err("File-drop window is already registered".into());
        }
        let previous = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
        if previous == 0 {
            return Err("Window procedure is unavailable".into());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let sink = Rc::new(DropSink {
            sender,
            wake,
            previous: Some(unsafe {
                std::mem::transmute::<
                    isize,
                    unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                >(previous)
            }),
        });
        let reference = Rc::into_raw(sink.clone());
        if let Err(error) =
            unsafe { SetPropW(hwnd, DROP_PROPERTY, Some(HANDLE(reference as *mut _))) }
        {
            unsafe { drop(Rc::from_raw(reference)) };
            return Err(format!("Register Shell file drops: {error}"));
        }
        if unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, drop_proc as *const () as isize) } == 0 {
            let error = windows::core::Error::from_thread();
            unsafe {
                let _ = RemovePropW(hwnd, DROP_PROPERTY);
                drop(Rc::from_raw(reference));
            }
            return Err(format!("Install Shell file-drop procedure: {error}"));
        }
        let mut guard = Self {
            hwnd,
            sink,
            receiver,
            filters_changed: [false; 3],
            accepted_before: unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32
                & WS_EX_ACCEPTFILES.0
                != 0,
        };
        for (index, message) in FILE_MESSAGES.into_iter().enumerate() {
            let mut status = CHANGEFILTERSTRUCT {
                cbSize: std::mem::size_of::<CHANGEFILTERSTRUCT>() as u32,
                ..Default::default()
            };
            unsafe { ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, Some(&mut status)) }
                .map_err(|error| format!("Allow Shell file-drop message {message:#x}: {error}"))?;
            guard.filters_changed[index] = status.ExtStatus == MSGFLTINFO_NONE;
        }
        unsafe { DragAcceptFiles(hwnd, true) };
        Ok(guard)
    }

    pub fn next_drop(&self) -> Option<WindowFileDrop> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for WindowFileDropGuard {
    fn drop(&mut self) {
        let reference = unsafe { GetPropW(self.hwnd, DROP_PROPERTY) }.0 as *const DropSink;
        if reference != Rc::as_ptr(&self.sink) {
            return;
        }
        unsafe {
            DragAcceptFiles(self.hwnd, self.accepted_before);
            for (message, changed) in FILE_MESSAGES.into_iter().zip(self.filters_changed) {
                if changed {
                    let _ = ChangeWindowMessageFilterEx(self.hwnd, message, MSGFLT_RESET, None);
                }
            }
            // Do not replace a later subclass's procedure. Its existing chain retains
            // our native reference until WM_NCDESTROY, without a dangling callback.
            if GetWindowLongPtrW(self.hwnd, GWLP_WNDPROC) == drop_proc as *const () as isize {
                SetWindowLongPtrW(
                    self.hwnd,
                    GWLP_WNDPROC,
                    self.sink.previous.unwrap() as usize as isize,
                );
                let _ = RemovePropW(self.hwnd, DROP_PROPERTY);
                drop(Rc::from_raw(reference));
            }
        }
    }
}

unsafe extern "system" fn drop_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let reference = unsafe { GetPropW(hwnd, DROP_PROPERTY) }.0 as *const DropSink;
    if reference.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let previous = unsafe { (*reference).previous };
    if message == WM_DROPFILES {
        // The wake callback may reenter window teardown; retain this invocation.
        unsafe { Rc::increment_strong_count(reference) };
        let sink = unsafe { Rc::from_raw(reference) };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let handle = DropHandle(HDROP(wparam.0 as *mut _));
            if lparam.0 != 0 {
                return;
            }
            if let Ok(drop) = read_drop(&handle, hwnd)
                && sink.sender.try_send(drop).is_ok()
            {
                (sink.wake)();
            }
        }));
        return LRESULT(0);
    }
    let result = unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) };
    if message == WM_NCDESTROY
        && unsafe { GetPropW(hwnd, DROP_PROPERTY) }.0 as *const DropSink == reference
    {
        unsafe {
            let _ = RemovePropW(hwnd, DROP_PROPERTY);
            drop(Rc::from_raw(reference));
        }
    }
    result
}

struct DropHandle(HDROP);
impl Drop for DropHandle {
    fn drop(&mut self) {
        unsafe { DragFinish(self.0) };
    }
}
struct LockedDrop(HGLOBAL);
impl Drop for LockedDrop {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.0);
        }
    }
}
fn read_drop(handle: &DropHandle, hwnd: HWND) -> Result<WindowFileDrop, &'static str> {
    let global = HGLOBAL(handle.0.0);
    let length = unsafe { GlobalSize(global) };
    if !(std::mem::size_of::<DROPFILES>()..=MAX_DROP_BYTES).contains(&length) {
        return Err("Invalid file-drop allocation");
    }
    let pointer = unsafe { GlobalLock(global) };
    if pointer.is_null() {
        return Err("File-drop memory is unavailable");
    }
    let _locked = LockedDrop(global);
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast(), length) };
    let mut drop = decode_drop(bytes)?;
    // Explorer's cross-integrity transfer can set the area flag while retaining
    // client coordinates. Reinterpreting it would shift the point a second time.
    // Use Shell's drop point directly, as GLFW does, and check the actual client.
    let mut point = POINT::default();
    let _ = unsafe { DragQueryPoint(handle.0, &mut point) };
    let mut client = RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client) }
        .map_err(|_| "File-drop client area is unavailable")?;
    if point.x < client.left
        || point.x >= client.right
        || point.y < client.top
        || point.y >= client.bottom
    {
        return Err("File drop is outside the client");
    }
    drop.position = [point.x, point.y];
    Ok(drop)
}

fn decode_drop(bytes: &[u8]) -> Result<WindowFileDrop, &'static str> {
    let header_size = std::mem::size_of::<DROPFILES>();
    if !(header_size..=MAX_DROP_BYTES).contains(&bytes.len()) {
        return Err("Invalid file-drop size");
    }
    let header = unsafe { bytes.as_ptr().cast::<DROPFILES>().read_unaligned() };
    let offset = header.pFiles as usize;
    if offset < header_size || offset >= bytes.len() {
        return Err("File drop has an invalid offset");
    }
    let data = &bytes[offset..];
    let units = if header.fWide.as_bool() {
        if !data.len().is_multiple_of(2) {
            return Err("Truncated UTF-16 file drop");
        }
        data.as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect::<Vec<_>>()
    } else {
        let length = unsafe { MultiByteToWideChar(CP_ACP, MB_PRECOMPOSED, data, None) };
        if length <= 0 {
            return Err("Invalid ANSI file drop");
        }
        let mut units = vec![0; length as usize];
        if unsafe { MultiByteToWideChar(CP_ACP, MB_PRECOMPOSED, data, Some(&mut units)) } != length
        {
            return Err("Invalid ANSI file drop");
        }
        units
    };
    let mut paths = Vec::new();
    let mut remaining = units.as_slice();
    loop {
        let end = remaining
            .iter()
            .position(|&unit| unit == 0)
            .ok_or("Unterminated file drop")?;
        if end == 0 {
            if paths.is_empty() {
                return Err("Empty file drop");
            }
            return Ok(WindowFileDrop {
                paths,
                position: [header.pt.x, header.pt.y],
            });
        }
        if end > MAX_PATH_UNITS || paths.len() == MAX_DROP_FILES {
            return Err("File-drop path limit exceeded");
        }
        let path = PathBuf::from(OsString::from_wide(&remaining[..end]));
        if !path.is_absolute() {
            return Err("File-drop path is not absolute");
        }
        paths.push(path);
        remaining = &remaining[end + 1..];
    }
}

#[cfg(test)]
mod tests;
