use super::identity::{self, handle};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
pub(super) fn request_close(pid: u32) -> Result<bool, String> {
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
        },
    };
    struct Request {
        pid: u32,
        result: Option<Result<(), String>>,
    }
    unsafe extern "system" fn visit(window: HWND, param: LPARAM) -> windows::core::BOOL {
        let request = unsafe { &mut *(param.0 as *mut Request) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, Some(&mut pid));
        }
        if pid == request.pid && unsafe { IsWindowVisible(window) }.as_bool() {
            request.result = Some(
                unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(|e| e.to_string()),
            );
            return false.into();
        }
        true.into()
    }
    let mut request = Request { pid, result: None };
    let enumerated =
        unsafe { EnumWindows(Some(visit), LPARAM((&mut request as *mut Request) as isize)) };
    match request.result {
        Some(result) => result.map(|()| true),
        None => {
            enumerated.map_err(|e| e.to_string())?;
            Ok(false)
        }
    }
}
use windows::Win32::{
    Foundation::{FILETIME, WAIT_OBJECT_0},
    System::Threading::*,
};

fn missing_process_error(code: windows::core::HRESULT) -> bool {
    code == windows::core::HRESULT::from_win32(
        windows::Win32::Foundation::ERROR_INVALID_PARAMETER.0,
    )
}

/// Positive exact-instance expiry only; denied or uncertain queries retain ownership.
pub fn process_instance_exited(pid: u32, token: u64) -> bool {
    if pid == 0 || token == 0 {
        return false;
    }
    let native = match unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    } {
        Ok(native) => native,
        Err(error) => return missing_process_error(error.code()),
    };
    let process = unsafe { OwnedHandle::from_raw_handle(native.0) };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe {
        GetProcessTimes(
            handle(&process),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .is_err()
    {
        return false;
    }
    let actual = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    actual != token || unsafe { WaitForSingleObject(handle(&process), 0) } == WAIT_OBJECT_0
}

pub struct ProcessState {
    pub pid: u32,
    pub creation_token: u64,
    pub affinity: usize,
    pub priority: crate::PriorityClass,
    pub elevated: bool,
    pub exited: bool,
}
pub struct ExactProcess {
    process: OwnedHandle,
    pid: u32,
    token: u64,
}
impl ExactProcess {
    pub fn open(
        pid: u32,
        token: u64,
        write: bool,
        terminate: bool,
        allow_elevated: bool,
    ) -> Result<Self, String> {
        if pid == 0 || token == 0 || pid == std::process::id() {
            return Err("A non-self exact process identity is required".into());
        }
        let rights = PROCESS_QUERY_LIMITED_INFORMATION
            | PROCESS_SYNCHRONIZE
            | if write {
                PROCESS_SET_INFORMATION
            } else {
                PROCESS_ACCESS_RIGHTS(0)
            }
            | if terminate {
                PROCESS_TERMINATE
            } else {
                PROCESS_ACCESS_RIGHTS(0)
            };
        let native = unsafe { OpenProcess(rights, false, pid) }.map_err(|e| e.to_string())?;
        let process = unsafe { OwnedHandle::from_raw_handle(native.0) };
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                handle(&process),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(|e| e.to_string())?;
        let actual = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
        if actual != token {
            return Err("Process instance changed; PID reuse is not an authorization".into());
        }
        let current = identity::open_token(unsafe { GetCurrentProcess() })?;
        let current = identity::inspect(handle(&current))?;
        let target = identity::open_token(handle(&process))?;
        let target = identity::inspect(handle(&target))?;
        let ordinary;
        let expected = if target.elevated {
            &current
        } else {
            ordinary = identity::launch_context(false)?;
            &ordinary.identity
        };
        if target.sid != current.sid
            || target.session != current.session
            || target.logon != expected.logon
            || (!allow_elevated && target.elevated)
        {
            return Err("Process belongs to a different user/session or needs administrator-process permission".into());
        }
        Ok(Self {
            process,
            pid,
            token,
        })
    }
    pub fn state(&self) -> Result<ProcessState, String> {
        let mut affinity = 0;
        let mut system = 0;
        unsafe { GetProcessAffinityMask(handle(&self.process), &mut affinity, &mut system) }
            .map_err(|e| e.to_string())?;
        let priority = match unsafe { GetPriorityClass(handle(&self.process)) } {
            v if v == IDLE_PRIORITY_CLASS.0 => crate::PriorityClass::Idle,
            v if v == BELOW_NORMAL_PRIORITY_CLASS.0 => crate::PriorityClass::BelowNormal,
            v if v == NORMAL_PRIORITY_CLASS.0 => crate::PriorityClass::Normal,
            v if v == ABOVE_NORMAL_PRIORITY_CLASS.0 => crate::PriorityClass::AboveNormal,
            v if v == HIGH_PRIORITY_CLASS.0 => crate::PriorityClass::High,
            v if v == REALTIME_PRIORITY_CLASS.0 => crate::PriorityClass::Realtime,
            _ => return Err("Cannot read process priority".into()),
        };
        let token = identity::open_token(handle(&self.process))?;
        Ok(ProcessState {
            pid: self.pid,
            creation_token: self.token,
            affinity,
            priority,
            elevated: identity::inspect(handle(&token))?.elevated,
            exited: unsafe { WaitForSingleObject(handle(&self.process), 0) } == WAIT_OBJECT_0,
        })
    }
    pub fn set_affinity(&self, cpus: &[usize]) -> Result<(), String> {
        let mask = super::launch::requested_mask(Some(cpus))?.unwrap();
        unsafe { SetProcessAffinityMask(handle(&self.process), mask) }
            .map_err(|e| e.to_string())?;
        if self.state()?.affinity != mask {
            return Err("Affinity readback differs from requested mask".into());
        }
        Ok(())
    }
    pub fn set_priority(&self, priority: crate::PriorityClass) -> Result<(), String> {
        if priority == crate::PriorityClass::Realtime {
            return Err("Realtime priority is not supported through the agent API".into());
        }
        unsafe {
            SetPriorityClass(
                handle(&self.process),
                super::super::common::transform_to_win_priority(priority),
            )
        }
        .map_err(|e| e.to_string())?;
        if self.state()?.priority != priority {
            return Err("Priority readback differs from requested value".into());
        }
        Ok(())
    }
    pub fn terminate(&self) -> Result<(), String> {
        unsafe { TerminateProcess(handle(&self.process), 1) }.map_err(|e| e.to_string())
    }
    pub fn request_close(&self) -> Result<bool, String> {
        if unsafe { WaitForSingleObject(handle(&self.process), 0) } == WAIT_OBJECT_0 {
            return Ok(false);
        }
        request_close(self.pid)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn scheduling_exit_probe_does_not_treat_denied_or_unknown_as_missing() {
        use windows::{Win32::Foundation::*, core::HRESULT};
        assert!(super::missing_process_error(HRESULT::from_win32(
            ERROR_INVALID_PARAMETER.0
        )));
        for code in [
            ERROR_ACCESS_DENIED.0,
            ERROR_NOT_ENOUGH_MEMORY.0,
            ERROR_INVALID_HANDLE.0,
        ] {
            assert!(!super::missing_process_error(HRESULT::from_win32(code)));
        }
    }
    #[test]
    fn scheduling_exit_probe_retains_live_exact_process_and_rejects_invalid_identity() {
        let pid = std::process::id();
        let token = crate::OS::get_process_instance_token(pid).unwrap();
        assert!(!super::process_instance_exited(pid, token));
        assert!(super::process_instance_exited(pid, token + 1));
        assert!(!super::process_instance_exited(0, token));
        assert!(!super::process_instance_exited(pid, 0));
    }
    #[test]
    fn exact_control_rejects_self_and_invalid_instance_before_any_write() {
        assert!(super::ExactProcess::open(std::process::id(), 1, true, false, true).is_err());
        assert!(super::ExactProcess::open(1, 0, true, true, true).is_err());
    }
}
