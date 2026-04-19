use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Globalization::{MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_ACCESS_RIGHTS,
    PROCESS_CREATION_FLAGS, REALTIME_PRIORITY_CLASS,
};
use windows::core::PCWSTR;

use crate::PriorityClass;

#[derive(Debug)]
pub(super) enum OsError {
    Win(windows::core::Error),
    Msg(String),
}

impl std::fmt::Display for OsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OsError::Win(e) => write!(f, "{e}"),
            OsError::Msg(s) => write!(f, "{s}"),
        }
    }
}

impl From<windows::core::Error> for OsError {
    fn from(e: windows::core::Error) -> Self {
        Self::Win(e)
    }
}

pub(super) struct HandleGuard(pub HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_invalid() {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

pub(super) struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() }
    }
}

pub(super) fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<HANDLE, OsError> {
    unsafe { Ok(OpenProcess(access, false, pid)?) }
}

pub(super) fn transform_to_win_priority(p: PriorityClass) -> PROCESS_CREATION_FLAGS {
    match p {
        PriorityClass::Idle => IDLE_PRIORITY_CLASS,
        PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        PriorityClass::High => HIGH_PRIORITY_CLASS,
        PriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
    }
}

pub(super) fn from_win_priority(p: u32) -> PriorityClass {
    match p {
        x if x == IDLE_PRIORITY_CLASS.0 => PriorityClass::Idle,
        x if x == BELOW_NORMAL_PRIORITY_CLASS.0 => PriorityClass::BelowNormal,
        x if x == NORMAL_PRIORITY_CLASS.0 => PriorityClass::Normal,
        x if x == ABOVE_NORMAL_PRIORITY_CLASS.0 => PriorityClass::AboveNormal,
        x if x == HIGH_PRIORITY_CLASS.0 => PriorityClass::High,
        x if x == REALTIME_PRIORITY_CLASS.0 => PriorityClass::Realtime,
        _ => PriorityClass::Normal,
    }
}

pub(super) fn to_wide_z(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain([0]).collect()
}

pub(super) fn to_wide_z_str(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0]).collect()
}

pub(super) fn expand_env(s: &str) -> String {
    let wide = to_wide_z_str(s);
    unsafe {
        let needed = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None);
        if needed == 0 {
            return s.to_string();
        }
        let mut buf = vec![0u16; needed as usize];
        let written = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf));
        if written == 0 {
            return s.to_string();
        }
        if let Some(pos) = buf.iter().position(|&c| c == 0) {
            buf.truncate(pos);
        }
        String::from_utf16_lossy(&buf)
    }
}

pub(super) fn decode_ansi(bytes: &[u8]) -> Option<String> {
    unsafe {
        let needed = MultiByteToWideChar(0, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if needed <= 0 {
            return None;
        }

        let mut buf = vec![0u16; needed as usize];
        let written =
            MultiByteToWideChar(0, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut buf));
        if written <= 0 {
            return None;
        }

        Some(String::from_utf16_lossy(&buf[..written as usize]))
    }
}
