use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetPriorityClass, GetProcessAffinityMask, GetProcessTimes,
    PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    SetPriorityClass, SetProcessAffinityMask,
};

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
            let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_INFORMATION;
            let handle = open_process(pid, access).or_else(|_| {
                open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_SET_INFORMATION)
            })?;
            let _hg = HandleGuard(handle);

            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)?;
            let instance_token =
                ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            if instance_token != expected_instance_token {
                return Err(OsError::Msg(
                    "process instance no longer matches the tracked PID".into(),
                ));
            }

            let mut current_mask = 0usize;
            let mut system_mask = 0usize;
            GetProcessAffinityMask(handle, &mut current_mask, &mut system_mask)?;
            let current_priority = GetPriorityClass(handle);
            if current_priority == 0 {
                return Err(OsError::Msg("GetPriorityClass returned 0".into()));
            }
            let previous_priority = from_win_priority(current_priority);
            let affinity_changed = current_mask != mask;
            let priority_changed = previous_priority != priority;
            if apply_changes && affinity_changed {
                SetProcessAffinityMask(handle, mask)?;
            }
            if apply_changes && priority_changed {
                SetPriorityClass(handle, transform_to_win_priority(priority))?;
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

            SetProcessAffinityMask(handle, mask)?;
            Ok(())
        })()
        .map_err(|e: OsError| format!("Failed to set affinity mask for process {}: {}", pid, e))
    }

    /// Sets the priority class for a process by PID.
    pub fn set_process_priority_by_pid(pid: u32, priority: PriorityClass) -> Result<(), String> {
        (|| unsafe {
            let handle = open_process(pid, PROCESS_SET_INFORMATION)?;
            let _hg = HandleGuard(handle);

            SetPriorityClass(handle, transform_to_win_priority(priority))?;
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
