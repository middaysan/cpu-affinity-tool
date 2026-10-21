use std::{
    ffi::c_void,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_ELEVATION,
            TOKEN_LINKED_TOKEN, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER, TokenElevation,
            TokenLinkedToken, TokenSessionId, TokenStatistics, TokenUser,
        },
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId},
    },
    core::PWSTR,
};

#[derive(PartialEq, Eq)]
pub(super) struct Identity {
    pub sid: String,
    pub session: u32,
    pub logon: u64,
    pub elevated: bool,
}
pub(super) fn handle(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}
pub(super) fn open_token(process: HANDLE) -> Result<OwnedHandle, String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|e| format!("Cannot inspect launch token: {e}"))?;
    Ok(unsafe { OwnedHandle::from_raw_handle(token.0) })
}
pub(super) fn inspect(token: HANDLE) -> Result<Identity, String> {
    unsafe {
        let mut length = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
        if !(size_of::<TOKEN_USER>() as u32..=65536).contains(&length) {
            return Err("Invalid launch user identity size".into());
        }
        let mut bytes = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        GetTokenInformation(
            token,
            TokenUser,
            Some(bytes.as_mut_ptr().cast()),
            length,
            &mut length,
        )
        .map_err(|e| e.to_string())?;
        let user = std::ptr::read_unaligned(bytes.as_ptr().cast::<TOKEN_USER>());
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid).map_err(|e| e.to_string())?;
        let sid_text = sid.to_string().map_err(|_| "Invalid launch user SID");
        let _ = LocalFree(Some(HLOCAL(sid.0.cast())));
        let mut session = 0u32;
        GetTokenInformation(
            token,
            TokenSessionId,
            Some((&mut session as *mut u32).cast::<c_void>()),
            size_of::<u32>() as u32,
            &mut length,
        )
        .map_err(|e| e.to_string())?;
        let mut stats = TOKEN_STATISTICS::default();
        GetTokenInformation(
            token,
            TokenStatistics,
            Some((&mut stats as *mut TOKEN_STATISTICS).cast()),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut length,
        )
        .map_err(|e| e.to_string())?;
        let mut elevation = TOKEN_ELEVATION::default();
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
        .map_err(|e| e.to_string())?;
        Ok(Identity {
            sid: sid_text?.into(),
            session,
            logon: ((stats.AuthenticationId.HighPart as u32 as u64) << 32)
                | stats.AuthenticationId.LowPart as u64,
            elevated: elevation.TokenIsElevated != 0,
        })
    }
}
pub(super) struct LaunchContext {
    pub token: OwnedHandle,
    pub shell: Option<OwnedHandle>,
    pub inherit_shell: bool,
    pub identity: Identity,
}
pub(super) fn launch_context(elevated: bool) -> Result<LaunchContext, String> {
    let current = open_token(unsafe { GetCurrentProcess() })?;
    let current_identity = inspect(handle(&current))?;
    if elevated {
        if !current_identity.elevated {
            return Err("Administrator launch requires an elevated application; there is no silent fallback".into());
        }
        return Ok(LaunchContext {
            token: current,
            shell: None,
            inherit_shell: false,
            identity: current_identity,
        });
    }
    let (token, needs_user_launch) = if current_identity.elevated {
        let mut linked = TOKEN_LINKED_TOKEN::default();
        let mut length = 0;
        unsafe {
            GetTokenInformation(
                handle(&current),
                TokenLinkedToken,
                Some((&mut linked as *mut TOKEN_LINKED_TOKEN).cast()),
                size_of::<TOKEN_LINKED_TOKEN>() as u32,
                &mut length,
            )
        }
        .map_err(|e| format!("No verified limited launch token: {e}"))?;
        (
            unsafe { OwnedHandle::from_raw_handle(linked.LinkedToken.0) },
            true,
        )
    } else {
        (current, false)
    };
    let identity = inspect(handle(&token))?;
    if identity.elevated
        || identity.sid != current_identity.sid
        || identity.session != current_identity.session
    {
        return Err("The ordinary launch token does not belong to this interactive user".into());
    }
    let shell = unsafe { GetShellWindow() };
    if shell.0.is_null() {
        return Err(
            "The interactive shell is unavailable; ordinary launch was not elevated as a fallback"
                .into(),
        );
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(shell, Some(&mut pid));
    }
    let rights = PROCESS_QUERY_LIMITED_INFORMATION
        | if needs_user_launch {
            windows::Win32::System::Threading::PROCESS_CREATE_PROCESS
                | windows::Win32::System::Threading::PROCESS_DUP_HANDLE
        } else {
            Default::default()
        };
    let process = unsafe { OpenProcess(rights, false, pid) }
        .map_err(|e| format!("Cannot verify interactive shell: {e}"))?;
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    // Linked tokens can be identification-only. They establish identity; an
    // ordinary elevated-parent launch inherits from the verified shell process.
    let mut shell_token = HANDLE::default();
    unsafe { OpenProcessToken(handle(&process), TOKEN_QUERY, &mut shell_token) }
        .map_err(|e| e.to_string())?;
    let shell_token = unsafe { OwnedHandle::from_raw_handle(shell_token.0) };
    if inspect(handle(&shell_token))? != identity {
        return Err(
            "Ordinary launch requires the same non-elevated interactive logon as Explorer".into(),
        );
    }
    Ok(LaunchContext {
        token,
        shell: Some(process),
        inherit_shell: needs_user_launch,
        identity,
    })
}
