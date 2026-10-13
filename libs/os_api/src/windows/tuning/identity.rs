use super::{code_error, native_error};
use crate::tuning::{
    TuningNativeError, TuningProcessIdentity, TuningProcessLifetime, TuningTokenIdentity,
};
use crate::windows::{OS, common::HandleGuard};
use std::{ffi::c_void, marker::PhantomData, mem::size_of, ptr, rc::Rc};
use windows::{
    Win32::{
        Foundation::{
            FILETIME, GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_FAILED, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, PSID, TOKEN_ELEVATION_TYPE,
            TOKEN_LINKED_TOKEN, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER, TokenElevationType,
            TokenElevationTypeFull, TokenLinkedToken, TokenSessionId, TokenStatistics, TokenUser,
        },
        System::Threading::{
            GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
    core::PWSTR,
};

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
    pub fn tuning_current_token_identity() -> Result<TuningTokenIdentity, TuningNativeError> {
        let token = current_token()?;
        token_identity(token.0)
    }

    /// Read the OS-linked limited token of the same elevated caller. This grants
    /// no impersonation or write authority and is not a public platform API.
    pub(crate) fn tuning_current_linked_token_identity(
        expected: &TuningTokenIdentity,
    ) -> Result<Option<TuningTokenIdentity>, TuningNativeError> {
        let token = current_token()?;
        if &token_identity(token.0)? != expected {
            return Err(TuningNativeError::Unsupported("caller token changed"));
        }
        let mut elevation = TOKEN_ELEVATION_TYPE::default();
        let mut length = 0;
        unsafe {
            GetTokenInformation(
                token.0,
                TokenElevationType,
                Some((&mut elevation as *mut TOKEN_ELEVATION_TYPE).cast()),
                size_of::<TOKEN_ELEVATION_TYPE>() as u32,
                &mut length,
            )
        }
        .map_err(|e| native_error("TokenElevationType", e))?;
        if elevation != TokenElevationTypeFull {
            return Ok(None);
        }
        let mut linked = TOKEN_LINKED_TOKEN::default();
        unsafe {
            GetTokenInformation(
                token.0,
                TokenLinkedToken,
                Some((&mut linked as *mut TOKEN_LINKED_TOKEN).cast()),
                size_of::<TOKEN_LINKED_TOKEN>() as u32,
                &mut length,
            )
        }
        .map_err(|e| native_error("TokenLinkedToken", e))?;
        let linked = HandleGuard(linked.LinkedToken);
        if linked.0.is_invalid() {
            return Err(TuningNativeError::Unsupported("invalid linked token"));
        }
        token_identity(linked.0).map(Some)
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

fn sid_string(sid: PSID) -> Result<String, TuningNativeError> {
    let mut value = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut value) }
        .map_err(|error| native_error("ConvertSidToStringSidW", error))?;
    let result = unsafe { value.to_string() }
        .map_err(|_| TuningNativeError::Unsupported("invalid native SID string"));
    unsafe {
        LocalFree(Some(HLOCAL(value.0.cast())));
    }
    result
}

#[cfg(test)]
mod token_tests {
    use super::*;
    #[test]
    fn linked_token_read_requires_the_exact_current_caller() {
        let caller = OS::tuning_current_token_identity().unwrap();
        let linked = OS::tuning_current_linked_token_identity(&caller).unwrap();
        if let Some(linked) = linked {
            assert_eq!(linked.sid(), caller.sid());
            assert_eq!(linked.session_id(), caller.session_id());
        }
        let mut changed = caller;
        changed.authentication_id ^= 1;
        assert!(OS::tuning_current_linked_token_identity(&changed).is_err());
    }
    #[test]
    fn current_process_has_exact_current_caller_identity() {
        let token = OS::tuning_current_token_identity().unwrap();
        let process = OS::tuning_process_identity(std::process::id()).unwrap();
        assert_eq!(process.token(), &token);
        assert_ne!(process.creation_time(), 0);
        assert!(process.image_path().ends_with(".exe"));
        assert!(!format!("{token:?}").contains(token.sid()));
    }
}
