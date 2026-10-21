//! Read-only interactive-session lifetime; no session or setting mutation.
use super::*;
use crate::gaming::session::{SessionObservation, expired, unknown};
use std::mem::offset_of;
use windows::Win32::{
    Foundation::{ERROR_CTX_WINSTATION_NOT_FOUND, ERROR_FILE_NOT_FOUND},
    Security::{
        Authentication::Identity::SECURITY_LOGON_SESSION_DATA,
        Authorization::ConvertSidToStringSidW,
    },
    System::RemoteDesktop::{
        WTS_SESSION_INFOW, WTSEnumerateSessionsW, WTSFreeMemory, WTSINFOW,
        WTSQuerySessionInformationW, WTSSessionInfo,
    },
};
use windows::core::PWSTR;

struct WtsBuffer(*mut c_void);
impl Drop for WtsBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WTSFreeMemory(self.0) };
        }
    }
}
struct LsaBuffer(*mut SECURITY_LOGON_SESSION_DATA);
impl Drop for LsaBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = LsaFreeReturnBuffer(self.0.cast());
            }
        }
    }
}
fn failure(reason: &'static str, value: windows::core::Error) -> GamingError {
    GamingError::Session {
        reason,
        code: Some(value.code().0 as u32 & 0xffff),
    }
}
fn listed(session_id: u32, current_id: u32) -> Result<bool, GamingError> {
    let mut data = ptr::null_mut::<WTS_SESSION_INFOW>();
    let mut count = 0;
    let result = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut data, &mut count) };
    let _buffer = WtsBuffer(data.cast());
    result.map_err(|e| failure("Cannot enumerate interactive sessions", e))?;
    if count == 0 || count > 4096 || data.is_null() {
        return Err(unknown("Incomplete interactive session inventory"));
    }
    let entries = unsafe { std::slice::from_raw_parts(data, count as usize) };
    if !entries.iter().any(|v| v.SessionId == current_id) {
        return Err(unknown(
            "Current interactive session is absent from the inventory",
        ));
    }
    Ok(entries.iter().any(|v| v.SessionId == session_id))
}
fn missing(code: u32, present: bool) -> Result<SessionObservation, GamingError> {
    if ![ERROR_FILE_NOT_FOUND.0, ERROR_CTX_WINSTATION_NOT_FOUND.0].contains(&code) {
        return Err(GamingError::Session {
            reason: "Cannot verify the interactive session",
            code: Some(code),
        });
    }
    if present {
        return Err(unknown(
            "Interactive session inventory changed during verification",
        ));
    }
    Ok(SessionObservation::Absent)
}
fn observe(session_id: u32, current_id: u32) -> Result<SessionObservation, GamingError> {
    let mut data = PWSTR::null();
    let mut length = 0;
    let result = unsafe {
        WTSQuerySessionInformationW(None, session_id, WTSSessionInfo, &mut data, &mut length)
    };
    let _buffer = WtsBuffer(data.0.cast());
    if let Err(error) = result {
        let value = error.code().0 as u32 & 0xffff;
        // A filtered enumeration alone is not absence. Require an explicit missing
        // result from the direct query as well; denial/transport errors stay unknown.
        if ![ERROR_FILE_NOT_FOUND.0, ERROR_CTX_WINSTATION_NOT_FOUND.0].contains(&value) {
            return missing(value, true);
        }
        return missing(value, listed(session_id, current_id)?);
    }
    if data.is_null() || length < size_of::<WTSINFOW>() as u32 {
        return Err(unknown("Incomplete interactive session data"));
    }
    let value = unsafe { &*data.0.cast::<WTSINFOW>() };
    let epoch = InteractiveSession {
        session_id: value.SessionId,
        logon_time: value.LogonTime,
    };
    if value.SessionId != session_id || !epoch.valid() || value.UserName[0] == 0 {
        return Err(unknown("Interactive session identity is unavailable"));
    }
    // Lock, RDP disconnect and reconnect retain the same instance.
    Ok(SessionObservation::Present(epoch))
}
pub(super) fn current(session_id: u32) -> Result<InteractiveSession, GamingError> {
    let first = observe(session_id, session_id)?;
    let second = observe(session_id, session_id)?;
    if first != second {
        return Err(unknown(
            "Current interactive session changed during verification",
        ));
    }
    match first {
        SessionObservation::Present(value) => Ok(value),
        SessionObservation::Absent => Err(unknown("Current interactive session is unavailable")),
    }
}
fn legacy_terminal(user: &UserIdentity) -> Result<Option<u32>, GamingError> {
    let luid = LUID {
        LowPart: user.logon as u32,
        HighPart: (user.logon >> 32) as i32,
    };
    let mut data = ptr::null_mut();
    let status = unsafe { LsaGetLogonSessionData(&luid, &mut data) };
    let _buffer = LsaBuffer(data);
    if super::logon_session_query_result(unsafe { LsaNtStatusToWinError(status) })? {
        return Ok(None);
    }
    if data.is_null() {
        return Err(unknown("Missing authentication session metadata"));
    }
    // Older or incomplete structures cannot supply lifetime evidence.
    if unsafe { (*data).Size }
        < (offset_of!(SECURITY_LOGON_SESSION_DATA, LogonTime) + size_of::<i64>()) as u32
    {
        return Err(unknown("Incomplete authentication session metadata"));
    }
    let id = unsafe { ptr::addr_of!((*data).LogonId).read() };
    let actual_id = (id.HighPart as u32 as u64) << 32 | id.LowPart as u64;
    let terminal = unsafe { ptr::addr_of!((*data).Session).read() };
    let kind = unsafe { ptr::addr_of!((*data).LogonType).read() };
    let sid = unsafe { ptr::addr_of!((*data).Sid).read() };
    if actual_id != user.logon
        || terminal == 0
        || ![2, 10, 11, 12].contains(&kind)
        || sid.is_invalid()
    {
        return Err(unknown(
            "Authentication record does not identify an interactive session",
        ));
    }
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }
        .map_err(|e| failure("Cannot verify the session account", e))?;
    let account = unsafe { text.to_string() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
    }
    if !account.is_ok_and(|account| account == user.sid) {
        return Err(unknown("Authentication session account differs"));
    }
    Ok(Some(terminal))
}
pub(super) fn has_ended(user: &UserIdentity, current_id: u32) -> Result<bool, GamingError> {
    let session_id = if let Some(saved) = user.session {
        if !saved.valid() {
            return Err(unknown("Invalid saved interactive session"));
        }
        saved.session_id
    } else {
        match legacy_terminal(user)? {
            Some(id) => id,
            None => return Ok(true),
        }
    };
    expired(
        user.session,
        observe(session_id, current_id)?,
        observe(session_id, current_id)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_requires_direct_not_found_and_consistent_inventory() {
        assert_eq!(missing(2, false), Ok(SessionObservation::Absent));
        assert_eq!(missing(7022, false), Ok(SessionObservation::Absent));
        assert!(missing(2, true).is_err());
        for error in [5, 1733, 1722, 87] {
            assert!(
                missing(error, false).is_err(),
                "query failure is not absence"
            );
        }
    }
}
