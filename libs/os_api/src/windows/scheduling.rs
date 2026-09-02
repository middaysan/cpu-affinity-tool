use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, FILETIME, HANDLE};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetPriorityClass, GetProcessAffinityMask, GetProcessInformation,
    GetProcessTimes, PROCESS_PROTECTION_LEVEL_INFORMATION, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROTECTION_LEVEL_NONE,
    ProcessProtectionLevelInfo, SetPriorityClass, SetProcessAffinityMask,
};
use windows::core::HRESULT;

use crate::{PriorityClass, ProcessSettingsApplyOutcome};

use super::OS;
use super::common::{
    HandleGuard, OsError, from_win_priority, open_process, transform_to_win_priority,
};

impl OS {
    /// Validates the process creation time and changes settings using the same handle.
    /// Holding a handle pins the process object, so a PID reuse cannot redirect this operation.
    pub fn apply_process_settings_if_instance(
        pid: u32,
        expected_instance_token: u64,
        mask: usize,
        priority: PriorityClass,
        apply_changes: bool,
    ) -> Result<ProcessSettingsApplyOutcome, String> {
        if mask == 0 {
            return Err("affinity mask is empty".to_string());
        }

        (|| unsafe {
            let handle = open_process_for_settings(pid, expected_instance_token)?;
            let _hg = HandleGuard(handle);

            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).map_err(
                |error| OsError::Msg(format!("GetProcessTimes failed for PID {pid}: {error}")),
            )?;
            let instance_token = process_instance_token(&created);
            if instance_token != expected_instance_token {
                return Err(OsError::Msg(
                    "process instance no longer matches the tracked PID".into(),
                ));
            }

            let mut current_mask = 0usize;
            let mut system_mask = 0usize;
            GetProcessAffinityMask(handle, &mut current_mask, &mut system_mask).map_err(
                |error| {
                    OsError::Msg(format!(
                        "GetProcessAffinityMask failed for PID {pid}: {error}"
                    ))
                },
            )?;
            let current_priority = GetPriorityClass(handle);
            if current_priority == 0 {
                return Err(OsError::Msg(format!(
                    "GetPriorityClass failed for PID {pid}: {}",
                    windows::core::Error::from_thread()
                )));
            }
            let previous_priority = from_win_priority(current_priority);
            let affinity_changed = current_mask != mask;
            let priority_changed = previous_priority != priority;
            if apply_changes && affinity_changed {
                SetProcessAffinityMask(handle, mask).map_err(|error| {
                    OsError::Msg(format!(
                        "SetProcessAffinityMask failed for PID {pid}: {error}"
                    ))
                })?;
            }
            if apply_changes && priority_changed {
                SetPriorityClass(handle, transform_to_win_priority(priority)).map_err(|error| {
                    OsError::Msg(format!("SetPriorityClass failed for PID {pid}: {error}"))
                })?;
            }

            Ok(ProcessSettingsApplyOutcome {
                previous_affinity: current_mask,
                previous_priority,
                affinity_changed,
                priority_changed,
            })
        })()
        .map_err(|error: OsError| format!("Failed to apply settings for process {pid}: {error}"))
    }

    /// Gets the current CPU affinity mask for a process.
    ///
    /// **Note:** On systems with more than 64 logical CPUs (Processor Groups),
    /// this function only returns the affinity mask for the current processor group.
    pub fn get_process_affinity(pid: u32) -> Result<usize, String> {
        (|| unsafe {
            let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| open_process(pid, PROCESS_QUERY_INFORMATION))?;
            let _hg = HandleGuard(handle);

            let mut process_mask: usize = 0;
            let mut system_mask: usize = 0;

            GetProcessAffinityMask(
                handle,
                &mut process_mask as *mut _,
                &mut system_mask as *mut _,
            )?;
            Ok(process_mask)
        })()
        .map_err(|e: OsError| format!("Failed to get affinity mask for process {}: {}", pid, e))
    }

    /// Gets the current priority class for a process.
    pub fn get_process_priority(pid: u32) -> Result<PriorityClass, String> {
        (|| unsafe {
            let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                .or_else(|_| open_process(pid, PROCESS_QUERY_INFORMATION))?;
            let _hg = HandleGuard(handle);

            let priority = GetPriorityClass(handle);
            if priority == 0 {
                return Err(OsError::Msg("GetPriorityClass returned 0".into()));
            }

            Ok(from_win_priority(priority))
        })()
        .map_err(|e: OsError| format!("Failed to get priority for process {}: {}", pid, e))
    }

    /// Sets the CPU affinity mask for a process by PID.
    ///
    /// **Note:** On systems with more than 64 logical CPUs (Processor Groups),
    /// this function only sets the affinity for the current processor group.
    pub fn set_process_affinity_by_pid(pid: u32, mask: usize) -> Result<(), String> {
        (|| unsafe {
            let handle = open_process(pid, PROCESS_SET_INFORMATION)?;
            let _hg = HandleGuard(handle);

            SetProcessAffinityMask(handle, mask).map_err(|error| {
                OsError::Msg(format!(
                    "SetProcessAffinityMask failed for PID {pid}: {error}"
                ))
            })?;
            Ok(())
        })()
        .map_err(|e: OsError| format!("Failed to set affinity mask for process {}: {}", pid, e))
    }

    /// Sets the priority class for a process by PID.
    pub fn set_process_priority_by_pid(pid: u32, priority: PriorityClass) -> Result<(), String> {
        (|| unsafe {
            let handle = open_process(pid, PROCESS_SET_INFORMATION)?;
            let _hg = HandleGuard(handle);

            SetPriorityClass(handle, transform_to_win_priority(priority)).map_err(|error| {
                OsError::Msg(format!("SetPriorityClass failed for PID {pid}: {error}"))
            })?;
            Ok(())
        })()
        .map_err(|e: OsError| format!("Failed to set priority for process {}: {}", pid, e))
    }

    /// Sets the priority class for the current process.
    pub fn set_current_process_priority(priority: PriorityClass) -> Result<(), String> {
        unsafe {
            let handle = GetCurrentProcess();
            SetPriorityClass(handle, transform_to_win_priority(priority))
                .map_err(|e| format!("Failed to set current process priority: {}", e))
        }
    }
}

fn open_process_for_settings(pid: u32, expected_instance_token: u64) -> Result<HANDLE, OsError> {
    let primary_access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_INFORMATION;
    match open_process(pid, primary_access) {
        Ok(handle) => Ok(handle),
        Err(primary_error) => {
            match open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_SET_INFORMATION) {
                Ok(handle) => Ok(handle),
                Err(fallback_error)
                    if is_access_denied(&primary_error)
                        && is_access_denied(&fallback_error)
                        && process_has_protection_level(pid, expected_instance_token) =>
                {
                    Err(OsError::Msg(format!(
                        "OpenProcess failed for PID {pid}: Windows protects this process; affinity and priority changes are not permitted"
                    )))
                }
                Err(fallback_error) => Err(fallback_error),
            }
        }
    }
}

fn is_access_denied(error: &OsError) -> bool {
    matches!(error, OsError::Win(error) if error.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0))
        || matches!(error, OsError::Operation { source, .. } if source.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0))
}

fn process_has_protection_level(pid: u32, expected_instance_token: u64) -> bool {
    unsafe {
        let Ok(handle) = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            return false;
        };
        let _guard = HandleGuard(handle);
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_err() {
            return false;
        }
        let instance_token = process_instance_token(&created);
        let mut info = PROCESS_PROTECTION_LEVEL_INFORMATION::default();

        GetProcessInformation(
            handle,
            ProcessProtectionLevelInfo,
            &mut info as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<PROCESS_PROTECTION_LEVEL_INFORMATION>() as u32,
        )
        .is_ok()
            && is_confirmed_protected_process(
                expected_instance_token,
                instance_token,
                info.ProtectionLevel,
            )
    }
}

fn process_instance_token(created: &FILETIME) -> u64 {
    ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64
}

fn is_confirmed_protected_process(
    expected_instance_token: u64,
    observed_instance_token: u64,
    protection_level: windows::Win32::System::Threading::PROCESS_PROTECTION_LEVEL,
) -> bool {
    expected_instance_token == observed_instance_token && is_protected_level(protection_level)
}

fn is_protected_level(level: windows::Win32::System::Threading::PROCESS_PROTECTION_LEVEL) -> bool {
    level != PROTECTION_LEVEL_NONE
}

#[cfg(test)]
mod tests {
    use super::{OsError, is_access_denied, is_confirmed_protected_process, is_protected_level};
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows::Win32::System::Threading::{PROTECTION_LEVEL_NONE, PROTECTION_LEVEL_WINTCB_LIGHT};
    use windows::core::{Error, HRESULT};

    #[test]
    fn access_denied_open_process_error_keeps_its_operation_context() {
        let error = OsError::Operation {
            operation: "OpenProcess",
            pid: 42,
            source: Error::from_hresult(HRESULT::from_win32(ERROR_ACCESS_DENIED.0)),
        };

        assert!(is_access_denied(&error));
        assert!(error.to_string().contains("OpenProcess failed for PID 42"));
    }

    #[test]
    fn protection_level_none_is_not_protected_but_wintcb_light_is() {
        assert!(!is_protected_level(PROTECTION_LEVEL_NONE));
        assert!(is_protected_level(PROTECTION_LEVEL_WINTCB_LIGHT));
    }

    #[test]
    fn protection_diagnostic_rejects_a_reused_pid_instance() {
        assert!(!is_confirmed_protected_process(
            100,
            101,
            PROTECTION_LEVEL_WINTCB_LIGHT
        ));
        assert!(!is_confirmed_protected_process(
            100,
            100,
            PROTECTION_LEVEL_NONE
        ));
        assert!(is_confirmed_protected_process(
            100,
            100,
            PROTECTION_LEVEL_WINTCB_LIGHT
        ));
    }
}
