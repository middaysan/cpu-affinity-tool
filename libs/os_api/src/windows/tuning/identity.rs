use super::{code_error, native_error, security::sid_string};
use crate::tuning::{
    MAX_TUNING_MACHINE_BYTES, TuningBootContinuity, TuningMachineIdentity, TuningNativeError,
    TuningProcessIdentity, TuningProcessLifetime, TuningTokenIdentity,
};
use crate::windows::{OS, common::HandleGuard};
use std::{ffi::c_void, marker::PhantomData, mem::size_of, ptr, rc::Rc};
use windows::{
    Storage::Streams::DataReader,
    System::Profile::SystemIdentification,
    Win32::{
        Foundation::{
            APPMODEL_ERROR_NO_PACKAGE, FILETIME, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
            TokenElevation, TokenSessionId, TokenStatistics, TokenUser,
        },
        Storage::Packaging::Appx::GetCurrentPackageFullName,
        System::{
            Threading::{
                GetCurrentProcess, GetCurrentThread, GetProcessId, GetProcessTimes, OpenProcess,
                OpenProcessToken, OpenThreadToken, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
                WaitForSingleObject,
            },
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
    },
    core::PWSTR,
};

fn machine_value(source: i32, bytes: Vec<u8>) -> Result<TuningMachineIdentity, TuningNativeError> {
    let source = match source {
        1 => crate::tuning::TuningMachineSource::Tpm,
        2 => crate::tuning::TuningMachineSource::Uefi,
        _ => {
            return Err(TuningNativeError::Unsupported(
                "system ID source is unavailable",
            ));
        }
    };
    if bytes.is_empty()
        || bytes.len() > MAX_TUNING_MACHINE_BYTES
        || bytes.iter().all(|byte| *byte == 0)
    {
        return Err(TuningNativeError::Unsupported("system ID byte length"));
    }
    Ok(TuningMachineIdentity { source, bytes })
}

struct RuntimeGuard;
impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

/// Opaque query/lifetime authority retained by one operation worker.
///
/// This guard grants no setting writes and cannot cross worker threads.
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<os_api::tuning::TuningProcessLifetimeGuard>();
/// ```
pub struct TuningProcessLifetimeGuard {
    process: HandleGuard,
    expected: TuningProcessIdentity,
    _worker_only: PhantomData<Rc<()>>,
}
impl std::fmt::Debug for TuningProcessLifetimeGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningProcessLifetimeGuard { native_authority: opaque }")
    }
}
impl TuningProcessLifetimeGuard {
    /// Poll the same retained native object without waiting. Query/wait failure
    /// is an error (unknown lifetime), never positive expiry.
    pub fn poll(&mut self) -> Result<TuningProcessLifetime, TuningNativeError> {
        let initial = poll_process(self.process.0)?;
        if initial == TuningProcessLifetime::PositivelyExited {
            // Initial construction authenticated this exact retained object.
            // No PID is reopened and terminal token queries are unnecessary.
            return Ok(initial);
        }
        require_expected_identity(&self.expected, process_identity(self.process.0))?;
        // If it exited during successful inspection, report the same object's
        // terminal state rather than a live result preceding that inspection.
        poll_process(self.process.0)
    }
}

fn classify_process_wait(wait: u32, code: u32) -> Result<TuningProcessLifetime, TuningNativeError> {
    if wait == WAIT_OBJECT_0.0 {
        Ok(TuningProcessLifetime::PositivelyExited)
    } else if wait == WAIT_TIMEOUT.0 {
        Ok(TuningProcessLifetime::VerifiedLive)
    } else if wait == WAIT_FAILED.0 {
        Err(code_error("WaitForSingleObject process lifetime", code))
    } else {
        Err(TuningNativeError::Unsupported(
            "unexpected process wait result",
        ))
    }
}
fn poll_process(process: HANDLE) -> Result<TuningProcessLifetime, TuningNativeError> {
    let wait = unsafe { WaitForSingleObject(process, 0) };
    let code = if wait == WAIT_FAILED {
        unsafe { GetLastError() }.0
    } else {
        0
    };
    classify_process_wait(wait.0, code)
}
fn require_expected_identity(
    expected: &TuningProcessIdentity,
    actual: Result<TuningProcessIdentity, TuningNativeError>,
) -> Result<(), TuningNativeError> {
    if &actual? == expected {
        Ok(())
    } else {
        Err(TuningNativeError::Unsupported("process identity mismatch"))
    }
}

impl OS {
    /// Open and verify the expected tuple through a single retained handle.
    /// No process query failure is classified as target expiry.
    pub fn tuning_open_process_lifetime(
        expected: &TuningProcessIdentity,
    ) -> Result<TuningProcessLifetimeGuard, TuningNativeError> {
        if expected.pid() == 0 {
            return Err(TuningNativeError::Unsupported("invalid PID"));
        }
        let process = HandleGuard(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    expected.pid(),
                )
            }
            .map_err(|e| native_error("OpenProcess lifetime", e))?,
        );
        require_expected_identity(expected, process_identity(process.0))?;
        let mut guard = TuningProcessLifetimeGuard {
            process,
            expected: expected.clone(),
            _worker_only: PhantomData,
        };
        // All checks, including any racing exit, use the retained object.
        guard.poll()?;
        Ok(guard)
    }
    pub fn tuning_machine_identity() -> Result<TuningMachineIdentity, TuningNativeError> {
        let mut package_length = 0;
        let package = unsafe { GetCurrentPackageFullName(&mut package_length, None) };
        if package != APPMODEL_ERROR_NO_PACKAGE {
            return Err(TuningNativeError::Unsupported(
                "publisher domain is not proven unpackaged",
            ));
        }
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
            .map_err(|e| native_error("RoInitialize", e))?;
        let _runtime = RuntimeGuard;
        let info = SystemIdentification::GetSystemIdForPublisher()
            .map_err(|e| native_error("GetSystemIdForPublisher", e))?;
        let source = info
            .Source()
            .map_err(|e| native_error("system ID source", e))?;
        let buffer = info.Id().map_err(|e| native_error("system ID buffer", e))?;
        let length = buffer
            .Length()
            .map_err(|e| native_error("system ID length", e))? as usize;
        if length == 0 || length > MAX_TUNING_MACHINE_BYTES {
            return Err(TuningNativeError::Unsupported("system ID byte length"));
        }
        let reader =
            DataReader::FromBuffer(&buffer).map_err(|e| native_error("system ID reader", e))?;
        let mut bytes = vec![0; length];
        reader
            .ReadBytes(&mut bytes)
            .map_err(|e| native_error("system ID byte copy", e))?;
        machine_value(source.0, bytes)
    }

    pub fn tuning_current_token_identity() -> Result<TuningTokenIdentity, TuningNativeError> {
        let token = current_token()?;
        token_identity(token.0)
    }

    pub fn tuning_process_identity(pid: u32) -> Result<TuningProcessIdentity, TuningNativeError> {
        if pid == 0 {
            return Err(TuningNativeError::Unsupported("invalid PID"));
        }
        let process = HandleGuard(
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
                .map_err(|e| native_error("OpenProcess identity", e))?,
        );
        process_identity(process.0)
    }

    pub const fn tuning_boot_continuity() -> TuningBootContinuity {
        TuningBootContinuity::Unavailable
    }
}

/// All tuple fields are obtained from this object, including its PID. This
/// helper never reopens by PID or treats a failed read as lifetime evidence.
fn process_identity(process: HANDLE) -> Result<TuningProcessIdentity, TuningNativeError> {
    let pid = unsafe { GetProcessId(process) };
    if pid == 0 {
        return Err(code_error(
            "GetProcessId identity",
            unsafe { GetLastError() }.0,
        ));
    }
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|e| native_error("OpenProcessToken target", e))?;
    let token = HandleGuard(token);
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
        .map_err(|e| native_error("GetProcessTimes identity", e))?;
    let creation_time = (creation.dwHighDateTime as u64) << 32 | creation.dwLowDateTime as u64;
    if creation_time == 0 || exit.dwLowDateTime != 0 || exit.dwHighDateTime != 0 {
        return Err(TuningNativeError::Unsupported(
            "target process is not proven live",
        ));
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|e| native_error("QueryFullProcessImageNameW identity", e))?;
    if length == 0 || length as usize >= path.len() {
        return Err(TuningNativeError::Unsupported("target image length"));
    }
    let image_path = String::from_utf16(&path[..length as usize])
        .map_err(|_| TuningNativeError::Unsupported("target image encoding"))?;
    Ok(TuningProcessIdentity {
        pid,
        creation_time,
        image_path,
        token: token_identity(token.0)?,
    })
}

#[cfg(test)]
#[path = "identity_lifetime_tests.rs"]
mod lifetime_tests;

pub(super) fn current_token() -> Result<HandleGuard, TuningNativeError> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| native_error("OpenProcessToken current", e))?;
    Ok(HandleGuard(token))
}

fn config_thread_context(opened: bool, error: u32) -> Result<(), TuningNativeError> {
    if !opened && error == 1008 {
        Ok(())
    } else {
        Err(TuningNativeError::Unsupported(
            "config thread impersonation/context",
        ))
    }
}
pub(super) struct ConfigPrincipal(TuningTokenIdentity);
impl ConfigPrincipal {
    pub(super) fn observe() -> Result<Self, TuningNativeError> {
        let mut thread_token = HANDLE::default();
        let opened =
            unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut thread_token) };
        let code = if let Err(error) = &opened {
            let raw = error.code().0 as u32;
            if raw & 0xffff0000 == 0x80070000 {
                raw & 0xffff
            } else {
                0
            }
        } else {
            0
        };
        if opened.is_ok() {
            drop(HandleGuard(thread_token));
        }
        config_thread_context(opened.is_ok(), code)?;
        let token = current_token()?;
        let identity = token_identity(token.0)?;
        if identity.sid().len() > 184 || !identity.sid().starts_with("S-1-") {
            return Err(TuningNativeError::Unsupported("config principal bounds"));
        }
        Ok(Self(identity))
    }
    pub(super) fn sid(&self) -> &str {
        self.0.sid()
    }
    pub(super) fn verify(&self) -> Result<(), TuningNativeError> {
        if Self::observe()?.0 == self.0 {
            Ok(())
        } else {
            Err(TuningNativeError::UnsafeStore("config principal changed"))
        }
    }
}
#[cfg(test)]
mod config_principal_tests {
    use super::*;
    #[test]
    fn p1b_only_positive_no_thread_token_allows_primary_context() {
        assert!(config_thread_context(false, 1008).is_ok());
        assert!(config_thread_context(true, 0).is_err());
        for code in [0, 5, 87] {
            assert!(config_thread_context(false, code).is_err());
        }
    }
}

pub(super) fn elevated() -> Result<bool, TuningNativeError> {
    let token = current_token()?;
    let mut value = TOKEN_ELEVATION::default();
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut value as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    }
    .map_err(|e| native_error("TokenElevation", e))?;
    Ok(value.TokenIsElevated != 0)
}

fn token_identity(token: HANDLE) -> Result<TuningTokenIdentity, TuningNativeError> {
    let mut length = 0;
    let size_error = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut length) };
    if size_error.is_ok() || length < size_of::<TOKEN_USER>() as u32 || length > 65536 {
        return Err(TuningNativeError::Unsupported("token user buffer length"));
    }
    let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            length,
            &mut length,
        )
    }
    .map_err(|e| native_error("TokenUser", e))?;
    let value = unsafe { ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let sid = sid_string(value.User.Sid)?;
    let mut session_id = 0u32;
    unsafe {
        GetTokenInformation(
            token,
            TokenSessionId,
            Some((&mut session_id as *mut u32).cast()),
            size_of::<u32>() as u32,
            &mut length,
        )
    }
    .map_err(|e| native_error("TokenSessionId", e))?;
    let mut stats = TOKEN_STATISTICS::default();
    unsafe {
        GetTokenInformation(
            token,
            TokenStatistics,
            Some((&mut stats as *mut TOKEN_STATISTICS).cast()),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut length,
        )
    }
    .map_err(|e| native_error("TokenStatistics", e))?;
    let authentication_id = (stats.AuthenticationId.HighPart as u32 as u64) << 32
        | stats.AuthenticationId.LowPart as u64;
    if authentication_id == 0 {
        return Err(TuningNativeError::Unsupported("zero AuthenticationId"));
    }
    Ok(TuningTokenIdentity {
        sid,
        session_id,
        authentication_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::{TuningMachineDomain, TuningMachineSource};
    #[test]
    fn machine_preserves_every_byte_and_source_without_fixed16_projection() {
        let bytes: Vec<_> = (0..37).collect();
        let identity = machine_value(1, bytes.clone()).unwrap();
        assert_eq!(identity.bytes(), bytes);
        assert_eq!(identity.source(), TuningMachineSource::Tpm);
        assert_eq!(identity.domain(), TuningMachineDomain::UnpackagedWin32);
        assert_ne!(identity, machine_value(2, bytes).unwrap());
    }
    #[test]
    fn machine_unknown_registry_and_invalid_lengths_are_unavailable() {
        for source in [0, 3, 999] {
            assert!(machine_value(source, vec![1; 32]).is_err());
        }
        assert!(machine_value(1, vec![]).is_err());
        assert!(machine_value(1, vec![1; MAX_TUNING_MACHINE_BYTES + 1]).is_err());
    }
    #[test]
    fn all_zero_native_machine_buffer_cannot_become_authority() {
        assert!(machine_value(1, vec![0; 32]).is_err());
    }
    #[test]
    fn actual_current_process_and_token_match_without_exposing_ids_in_debug() {
        let token = OS::tuning_current_token_identity().unwrap();
        let process = OS::tuning_process_identity(std::process::id()).unwrap();
        assert_eq!(process.token(), &token);
        assert_ne!(process.creation_time(), 0);
        assert!(process.image_path().ends_with(".exe"));
        assert!(!format!("{token:?}").contains(token.sid()));
        assert_eq!(
            OS::tuning_boot_continuity(),
            TuningBootContinuity::Unavailable
        );
    }
}
