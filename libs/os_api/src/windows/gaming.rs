//! Narrow, conditional Windows operations. All process work uses one retained handle.
pub(crate) mod cpu_power;
mod session;
use super::{OS, common::HandleGuard};
use crate::gaming::*;
use crate::tuning::TuningNativeError;
use std::{ffi::c_void, mem::size_of, ptr};
use windows::{
    Win32::{
        Foundation::{
            ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS,
            ERROR_NO_SUCH_LOGON_SESSION, ERROR_NO_TOKEN, FILETIME, FreeLibrary, GetLastError,
            HANDLE, HLOCAL, HMODULE, LUID, LocalFree,
        },
        Security::{
            Authentication::Identity::{
                LsaFreeReturnBuffer, LsaGetLogonSessionData, LsaNtStatusToWinError,
            },
            TOKEN_QUERY,
        },
        System::{
            LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            Power::{
                ACCESS_SCHEME, PowerEnumerate, PowerGetActiveScheme, PowerReadFriendlyName,
                PowerSetActiveScheme,
            },
            SystemInformation::{
                CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
            },
            Threading::{
                GetCurrentThread, GetPriorityClass, GetProcessDefaultCpuSets,
                GetProcessInformation, GetProcessTimes, OpenProcess, OpenThreadToken,
                PROCESS_CREATION_FLAGS, PROCESS_NAME_WIN32,
                PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_STATE,
                PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION, ProcessPowerThrottling,
                QueryFullProcessImageNameW, SetPriorityClass, SetProcessDefaultCpuSets,
                SetProcessInformation,
            },
        },
        UI::{
            Accessibility::{FILTERKEYS, STICKYKEYS, STICKYKEYS_FLAGS, TOGGLEKEYS},
            WindowsAndMessaging::{
                GetShellWindow, GetWindowThreadProcessId, SPI_GETFILTERKEYS, SPI_GETMOUSE,
                SPI_GETSTICKYKEYS, SPI_GETTOGGLEKEYS, SPI_SETFILTERKEYS, SPI_SETMOUSE,
                SPI_SETSTICKYKEYS, SPI_SETTOGGLEKEYS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
                SystemParametersInfoW,
            },
        },
    },
    core::{GUID, PCSTR, PWSTR, w},
};

fn code(operation: &'static str, value: u32) -> GamingError {
    if value == ERROR_ACCESS_DENIED.0 {
        GamingError::Denied {
            operation,
            code: value,
        }
    } else {
        GamingError::Native {
            operation,
            code: value,
        }
    }
}
fn win(operation: &'static str, error: windows::core::Error) -> GamingError {
    code(operation, error.code().0 as u32 & 0xffff)
}
fn native_identity(error: TuningNativeError) -> GamingError {
    match error {
        TuningNativeError::Denied {
            operation,
            code: value,
        }
        | TuningNativeError::Io {
            operation,
            code: value,
        } => code(operation, value),
        TuningNativeError::Unsupported(reason) => GamingError::Unsupported(reason),
        _ => GamingError::Unsupported("Cannot verify native user context"),
    }
}
fn success(operation: &'static str, value: u32) -> Result<(), GamingError> {
    if value == 0 {
        Ok(())
    } else {
        Err(code(operation, value))
    }
}

// A split UAC logon is accepted only through its actual OS-linked token.
fn caller_matches_shell(
    caller: &crate::tuning::TuningTokenIdentity,
    shell: &crate::tuning::TuningTokenIdentity,
    linked: Option<&crate::tuning::TuningTokenIdentity>,
) -> bool {
    caller == shell
        || (caller.sid() == shell.sid()
            && caller.session_id() == shell.session_id()
            && linked == Some(shell))
}

/// Cross-account elevation, missing shells and impersonation cannot write user settings.
fn interactive_context() -> Result<(UserIdentity, u32), GamingError> {
    let mut thread_token = HANDLE::default();
    match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut thread_token) } {
        Ok(()) => {
            let _token = HandleGuard(thread_token);
            return Err(GamingError::Unsupported(
                "Per-user settings are unavailable while impersonating",
            ));
        }
        Err(error) if error.code().0 as u32 & 0xffff == ERROR_NO_TOKEN.0 => {}
        Err(error) => return Err(win("OpenThreadToken", error)),
    }
    let caller = OS::tuning_current_token_identity().map_err(native_identity)?;
    let shell = unsafe { GetShellWindow() };
    if shell.is_invalid() {
        return Err(GamingError::Unsupported(
            "An interactive Explorer shell is required",
        ));
    }
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(shell, Some(&mut pid)) } == 0 || pid == 0 {
        return Err(GamingError::Unsupported(
            "Cannot identify the interactive Explorer shell",
        ));
    }
    let first = OS::tuning_process_identity(pid).map_err(native_identity)?;
    let filename = std::path::Path::new(first.image_path())
        .file_name()
        .and_then(|s| s.to_str());
    if !filename.is_some_and(|s| s.eq_ignore_ascii_case("explorer.exe")) {
        return Err(GamingError::Unsupported(
            "The interactive shell is not Explorer",
        ));
    }
    let linked = if &caller != first.token() {
        OS::tuning_current_linked_token_identity(&caller).map_err(native_identity)?
    } else {
        None
    };
    if !caller_matches_shell(&caller, first.token(), linked.as_ref()) {
        return Err(GamingError::Denied {
            operation: if caller.sid() != first.token().sid() {
                "App and Explorer Windows accounts differ"
            } else if caller.session_id() != first.token().session_id() {
                "App and Explorer desktop sessions differ"
            } else {
                "App and its UAC-linked token do not match the Explorer sign-in"
            },
            code: ERROR_ACCESS_DENIED.0,
        });
    }
    let second = OS::tuning_process_identity(pid).map_err(native_identity)?;
    let mut final_pid = 0;
    unsafe { GetWindowThreadProcessId(shell, Some(&mut final_pid)) };
    if first != second
        || unsafe { GetShellWindow() } != shell
        || final_pid != pid
        || OS::tuning_current_token_identity().map_err(native_identity)? != caller
    {
        return Err(GamingError::Changed);
    }
    Ok((
        UserIdentity {
            sid: caller.sid().to_string(),
            // Persist the same desktop identity before and after UAC elevation.
            logon: first.token().authentication_id(),
            session: None,
        },
        first.token().session_id(),
    ))
}
pub fn interactive_user() -> Result<UserIdentity, GamingError> {
    interactive_context().map(|(user, _)| user)
}
pub fn interactive_session_user() -> Result<UserIdentity, GamingError> {
    let before = interactive_context()?;
    let epoch = session::current(before.1)?;
    if interactive_context()? != before {
        return Err(crate::gaming::session::unknown(
            "Desktop context changed during session verification",
        ));
    }
    Ok(UserIdentity {
        session: Some(epoch),
        ..before.0
    })
}
pub(super) fn verify_user(expected: &UserIdentity) -> Result<(), GamingError> {
    let actual = if expected.session.is_some() {
        interactive_session_user()
    } else {
        interactive_user()
    };
    verified_session_user(expected, actual).map(|_| ())
}

fn logon_session_query_result(error: u32) -> Result<bool, GamingError> {
    match error {
        0 => Ok(false),
        value if value == ERROR_NO_SUCH_LOGON_SESSION.0 => Ok(true),
        value => Err(code("LsaGetLogonSessionData", value)),
    }
}

/// WTS identifies the interactive instance; surviving LSA tokens do not keep it alive.
pub fn session_has_ended(expected: &UserIdentity) -> Result<bool, GamingError> {
    let before = interactive_context()?;
    let result = session_expiry_with(expected, &before.0, |_| {
        session::has_ended(expected, before.1)
    });
    if interactive_context()? != before {
        return Err(crate::gaming::session::unknown(
            "Desktop context changed during lifetime verification",
        ));
    }
    result
}

#[cfg(test)]
mod session_expiry_tests {
    use super::*;
    #[test]
    fn native_session_expiry_does_not_treat_read_errors_as_absence() {
        assert_eq!(logon_session_query_result(0), Ok(false));
        assert_eq!(
            logon_session_query_result(ERROR_NO_SUCH_LOGON_SESSION.0),
            Ok(true)
        );
        for error in [ERROR_ACCESS_DENIED.0, 87, 8] {
            assert!(logon_session_query_result(error).is_err());
        }
    }

    #[test]
    #[ignore = "Requires an interactive Windows desktop; reads logon lifetime only"]
    fn native_session_expiry_reads_current_and_absent_logon_without_mutation() {
        let current = interactive_user().unwrap();
        assert!(!session_has_ended(&current).unwrap());
        let absent = UserIdentity {
            logon: u64::MAX,
            ..current
        };
        assert!(session_has_ended(&absent).unwrap());
    }

    #[test]
    #[ignore = "Requires an explicitly verified logged-off desktop LUID; read-only"]
    fn native_retained_lsa_logon_is_retired_after_interactive_logoff() {
        let current = interactive_user().unwrap();
        let old = UserIdentity {
            logon: std::env::var("CPU_AFFINITY_TEST_OLD_LOGON")
                .expect("explicit old same-account LUID")
                .parse()
                .unwrap(),
            ..current.clone()
        };
        assert_ne!(old, current, "never use the current desktop as the fixture");
        assert!(session_has_ended(&old).unwrap());
    }
}
pub(super) fn admit_account(expected: &UserIdentity) -> Result<UserIdentity, GamingError> {
    // Old recovery targets name a previous logon. Admission still obtains the
    // fully checked current caller/Explorer identity and pins it for this operation.
    admitted_account_user(expected, interactive_user())
}
fn with_user<T>(
    expected: &UserIdentity,
    operation: impl FnOnce() -> Result<T, GamingError>,
) -> Result<T, GamingError> {
    verify_user(expected)?;
    let value = operation()?;
    verify_user(expected)?;
    Ok(value)
}
fn with_account<T>(
    expected: &UserIdentity,
    operation: impl FnOnce() -> Result<T, GamingError>,
) -> Result<T, GamingError> {
    let current = admit_account(expected)?;
    let value = operation()?;
    verify_user(&current)?;
    Ok(value)
}
fn guid(value: GuidValue) -> GUID {
    GUID::from_u128(u128::from_be_bytes(value.0))
}
fn guid_value(value: GUID) -> GuidValue {
    GuidValue(value.to_u128().to_be_bytes())
}

fn active_scheme() -> Result<GuidValue, GamingError> {
    let mut raw = ptr::null_mut();
    success(
        "PowerGetActiveScheme",
        unsafe { PowerGetActiveScheme(None, &mut raw) }.0,
    )?;
    if raw.is_null() {
        return Err(GamingError::Unsupported(
            "PowerGetActiveScheme returned no GUID",
        ));
    }
    // Windows allocates this GUID with LocalAlloc; copy it before releasing it.
    let value = guid_value(unsafe { *raw });
    unsafe { LocalFree(Some(HLOCAL(raw.cast()))) };
    Ok(value)
}
pub fn read_power_plan(user: &UserIdentity) -> Result<GuidValue, GamingError> {
    with_account(user, active_scheme)
}
pub fn power_schemes(user: &UserIdentity) -> Result<Vec<PowerScheme>, GamingError> {
    with_account(user, || {
        let mut schemes = Vec::new();
        for index in 0..128 {
            let mut value = GUID::zeroed();
            let mut length = size_of::<GUID>() as u32;
            let result = unsafe {
                PowerEnumerate(
                    None,
                    None,
                    None,
                    ACCESS_SCHEME,
                    index,
                    Some((&mut value as *mut GUID).cast()),
                    &mut length,
                )
            };
            if result == ERROR_NO_MORE_ITEMS {
                return Ok(schemes);
            }
            success("PowerEnumerate", result.0)?;
            if length != size_of::<GUID>() as u32 {
                return Err(GamingError::Unsupported("Invalid power scheme GUID length"));
            }
            let mut name_length = 0;
            let sizing = unsafe {
                PowerReadFriendlyName(None, Some(&value), None, None, None, &mut name_length)
            };
            if sizing.0 != 0 && sizing != ERROR_INSUFFICIENT_BUFFER {
                return Err(code("PowerReadFriendlyName", sizing.0));
            }
            if !(2..=8192).contains(&name_length) || name_length % 2 != 0 {
                return Err(GamingError::Unsupported("Invalid power scheme name length"));
            }
            let mut name = vec![0u16; name_length as usize / 2];
            let capacity = name_length;
            success(
                "PowerReadFriendlyName",
                unsafe {
                    PowerReadFriendlyName(
                        None,
                        Some(&value),
                        None,
                        None,
                        Some(name.as_mut_ptr().cast()),
                        &mut name_length,
                    )
                }
                .0,
            )?;
            if name_length > capacity || name_length % 2 != 0 {
                return Err(GamingError::Changed);
            }
            name.truncate(name_length as usize / 2);
            if name.last() == Some(&0) {
                name.pop();
            }
            let name = String::from_utf16(&name)
                .map_err(|_| GamingError::Unsupported("Invalid power scheme name"))?;
            schemes.push(PowerScheme {
                id: guid_value(value),
                name,
            });
        }
        Err(GamingError::Unsupported("Too many power schemes"))
    })
}
pub fn write_power_plan(
    user: &UserIdentity,
    expected: GuidValue,
    desired: GuidValue,
) -> Result<(), GamingError> {
    let current = admit_account(user)?;
    if !power_schemes(&current)?
        .iter()
        .any(|scheme| scheme.id == desired)
    {
        return Err(GamingError::Unsupported(
            "The selected power scheme no longer exists",
        ));
    }
    if active_scheme()? != expected {
        return Err(GamingError::Changed);
    }
    verify_user(&current)?;
    success(
        "PowerSetActiveScheme",
        unsafe { PowerSetActiveScheme(None, Some(&guid(desired))) }.0,
    )?;
    let actual = read_power_plan(&current)?;
    verify_user(&current)?;
    if actual != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}

type GetMode = unsafe extern "system" fn(*mut GUID) -> u32;
type SetMode = unsafe extern "system" fn(*const GUID) -> u32;
struct ModeApi {
    module: HMODULE,
    get_ac: GetMode,
    get_dc: GetMode,
    set_ac: SetMode,
    set_dc: SetMode,
}
impl Drop for ModeApi {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.module);
        }
    }
}
impl ModeApi {
    fn load() -> Result<Self, GamingError> {
        let module =
            unsafe { LoadLibraryExW(w!("powrprof.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .map_err(|e| win("LoadLibraryExW powrprof", e))?;
        let get_ac = unsafe {
            GetProcAddress(
                module,
                PCSTR(c"PowerGetUserConfiguredACPowerMode".as_ptr().cast()),
            )
        };
        let get_dc = unsafe {
            GetProcAddress(
                module,
                PCSTR(c"PowerGetUserConfiguredDCPowerMode".as_ptr().cast()),
            )
        };
        let set_ac = unsafe {
            GetProcAddress(
                module,
                PCSTR(c"PowerSetUserConfiguredACPowerMode".as_ptr().cast()),
            )
        };
        let set_dc = unsafe {
            GetProcAddress(
                module,
                PCSTR(c"PowerSetUserConfiguredDCPowerMode".as_ptr().cast()),
            )
        };
        let (Some(get_ac), Some(get_dc), Some(set_ac), Some(set_dc)) =
            (get_ac, get_dc, set_ac, set_dc)
        else {
            unsafe {
                let _ = FreeLibrary(module);
            }
            return Err(GamingError::Unsupported(
                "Windows 11 user-configured AC/DC Power Mode APIs are unavailable",
            ));
        };
        // These four documented powrprof exports have exactly DWORD(GUID*) signatures.
        Ok(unsafe {
            Self {
                module,
                get_ac: std::mem::transmute::<unsafe extern "system" fn() -> isize, GetMode>(
                    get_ac,
                ),
                get_dc: std::mem::transmute::<unsafe extern "system" fn() -> isize, GetMode>(
                    get_dc,
                ),
                set_ac: std::mem::transmute::<unsafe extern "system" fn() -> isize, SetMode>(
                    set_ac,
                ),
                set_dc: std::mem::transmute::<unsafe extern "system" fn() -> isize, SetMode>(
                    set_dc,
                ),
            }
        })
    }
    fn read(&self, source: PowerSource) -> Result<GuidValue, GamingError> {
        let mut value = GUID::zeroed();
        success("PowerGetUserConfiguredPowerMode", unsafe {
            match source {
                PowerSource::Ac => (self.get_ac)(&mut value),
                PowerSource::Dc => (self.get_dc)(&mut value),
            }
        })?;
        Ok(guid_value(value))
    }
}
pub fn read_power_mode(user: &UserIdentity, source: PowerSource) -> Result<GuidValue, GamingError> {
    with_account(user, || ModeApi::load()?.read(source))
}
pub fn write_power_mode(
    user: &UserIdentity,
    source: PowerSource,
    expected: GuidValue,
    desired: GuidValue,
) -> Result<(), GamingError> {
    if ![
        POWER_MODE_BALANCED,
        POWER_MODE_EFFICIENCY,
        POWER_MODE_PERFORMANCE,
    ]
    .contains(&desired)
    {
        return Err(GamingError::InvalidValue);
    }
    let current = admit_account(user)?;
    let api = ModeApi::load()?;
    if api.read(source)? != expected {
        return Err(GamingError::Changed);
    }
    verify_user(&current)?;
    let desired_guid = guid(desired);
    success("PowerSetUserConfiguredPowerMode", unsafe {
        match source {
            PowerSource::Ac => (api.set_ac)(&desired_guid),
            PowerSource::Dc => (api.set_dc)(&desired_guid),
        }
    })?;
    verify_user(&current)?;
    let actual = api.read(source)?;
    verify_user(&current)?;
    if actual != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}

fn mouse() -> Result<MouseParameters, GamingError> {
    let mut value = [0i32; 3];
    unsafe {
        SystemParametersInfoW(
            SPI_GETMOUSE,
            0,
            Some(value.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| win("SPI_GETMOUSE", e))?;
    Ok(MouseParameters(value))
}
pub fn read_mouse(user: &UserIdentity) -> Result<MouseParameters, GamingError> {
    with_user(user, mouse)
}
pub fn write_mouse(
    user: &UserIdentity,
    expected: MouseParameters,
    mut desired: MouseParameters,
) -> Result<(), GamingError> {
    verify_user(user)?;
    if mouse()? != expected {
        return Err(GamingError::Changed);
    }
    verify_user(user)?;
    // Session-only SPI flags avoid changing the persisted Control Panel preferences.
    unsafe {
        SystemParametersInfoW(
            SPI_SETMOUSE,
            0,
            Some(desired.0.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| win("SPI_SETMOUSE", e))?;
    if read_mouse(user)? != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}
enum AccessibilityValue {
    Sticky(STICKYKEYS),
    Filter(FILTERKEYS),
    Toggle(TOGGLEKEYS),
}
impl AccessibilityValue {
    fn flags(&self) -> u32 {
        match self {
            Self::Sticky(v) => v.dwFlags.0,
            Self::Filter(v) => v.dwFlags,
            Self::Toggle(v) => v.dwFlags,
        }
    }
    fn set_hotkeys(&mut self, desired: u32) -> Result<(), GamingError> {
        let flags = merge_hotkeys(self.flags(), desired)?;
        match self {
            Self::Sticky(v) => v.dwFlags = STICKYKEYS_FLAGS(flags),
            Self::Filter(v) => v.dwFlags = flags,
            Self::Toggle(v) => v.dwFlags = flags,
        }
        Ok(())
    }
    fn write(&mut self) -> Result<(), GamingError> {
        let (action, length, value) = match self {
            Self::Sticky(v) => (
                SPI_SETSTICKYKEYS,
                size_of::<STICKYKEYS>(),
                (v as *mut STICKYKEYS).cast::<c_void>(),
            ),
            Self::Filter(v) => (
                SPI_SETFILTERKEYS,
                size_of::<FILTERKEYS>(),
                (v as *mut FILTERKEYS).cast::<c_void>(),
            ),
            Self::Toggle(v) => (
                SPI_SETTOGGLEKEYS,
                size_of::<TOGGLEKEYS>(),
                (v as *mut TOGGLEKEYS).cast::<c_void>(),
            ),
        };
        unsafe {
            SystemParametersInfoW(
                action,
                length as u32,
                Some(value),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .map_err(|e| win("SPI_SET accessibility", e))
    }
}
fn accessibility(feature: AccessibilityFeature) -> Result<AccessibilityValue, GamingError> {
    let (mut value, action) = match feature {
        AccessibilityFeature::Sticky => (
            AccessibilityValue::Sticky(STICKYKEYS {
                cbSize: size_of::<STICKYKEYS>() as u32,
                ..Default::default()
            }),
            SPI_GETSTICKYKEYS,
        ),
        AccessibilityFeature::Filter => (
            AccessibilityValue::Filter(FILTERKEYS {
                cbSize: size_of::<FILTERKEYS>() as u32,
                ..Default::default()
            }),
            SPI_GETFILTERKEYS,
        ),
        AccessibilityFeature::Toggle => (
            AccessibilityValue::Toggle(TOGGLEKEYS {
                cbSize: size_of::<TOGGLEKEYS>() as u32,
                ..Default::default()
            }),
            SPI_GETTOGGLEKEYS,
        ),
    };
    let (length, data) = match &mut value {
        AccessibilityValue::Sticky(v) => (
            size_of::<STICKYKEYS>(),
            (v as *mut STICKYKEYS).cast::<c_void>(),
        ),
        AccessibilityValue::Filter(v) => (
            size_of::<FILTERKEYS>(),
            (v as *mut FILTERKEYS).cast::<c_void>(),
        ),
        AccessibilityValue::Toggle(v) => (
            size_of::<TOGGLEKEYS>(),
            (v as *mut TOGGLEKEYS).cast::<c_void>(),
        ),
    };
    unsafe {
        SystemParametersInfoW(
            action,
            length as u32,
            Some(data),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| win("SPI_GET accessibility", e))?;
    Ok(value)
}
pub fn read_accessibility(
    user: &UserIdentity,
    feature: AccessibilityFeature,
) -> Result<AccessibilityState, GamingError> {
    with_user(user, || {
        let flags = accessibility(feature)?.flags();
        Ok(AccessibilityState {
            enabled: flags & 1 != 0,
            hotkey_flags: flags & ACCESSIBILITY_HOTKEY_MASK,
        })
    })
}
#[cfg(feature = "tuning-test-controls")]
pub fn read_user_setting_validation(
    user: &UserIdentity,
) -> Result<UserSettingValidation, GamingError> {
    use windows::Win32::UI::WindowsAndMessaging::SPI_GETMOUSESPEED;
    with_user(user, || {
        let mut mouse_speed = 0i32;
        unsafe {
            SystemParametersInfoW(
                SPI_GETMOUSESPEED,
                0,
                Some((&mut mouse_speed as *mut i32).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .map_err(|error| win("SPI_GETMOUSESPEED validation", error))?;
        let AccessibilityValue::Sticky(sticky) = accessibility(AccessibilityFeature::Sticky)?
        else {
            unreachable!()
        };
        let AccessibilityValue::Filter(filter) = accessibility(AccessibilityFeature::Filter)?
        else {
            unreachable!()
        };
        let AccessibilityValue::Toggle(toggle) = accessibility(AccessibilityFeature::Toggle)?
        else {
            unreachable!()
        };
        Ok(UserSettingValidation {
            mouse_speed,
            sticky_flags: sticky.dwFlags.0,
            filter_flags: filter.dwFlags,
            filter_wait_ms: filter.iWaitMSec,
            filter_delay_ms: filter.iDelayMSec,
            filter_repeat_ms: filter.iRepeatMSec,
            filter_bounce_ms: filter.iBounceMSec,
            toggle_flags: toggle.dwFlags,
        })
    })
}
pub fn write_accessibility(
    user: &UserIdentity,
    feature: AccessibilityFeature,
    expected: u32,
    desired: u32,
) -> Result<(), GamingError> {
    verify_user(user)?;
    let mut current = accessibility(feature)?;
    if current.flags() & 1 != 0 {
        return Err(GamingError::Unsupported(
            "The accessibility feature is already enabled; its hotkeys are preserved",
        ));
    }
    if current.flags() & ACCESSIBILITY_HOTKEY_MASK != expected {
        return Err(GamingError::Changed);
    }
    current.set_hotkeys(desired)?;
    verify_user(user)?;
    current.write()?;
    if read_accessibility(user, feature)?.hotkey_flags != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}

fn verify_process(handle: HANDLE, expected: &ProcessIdentity) -> Result<(), GamingError> {
    let mut created = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) }
        .map_err(|e| win("GetProcessTimes", e))?;
    let token = (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64;
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|e| win("QueryFullProcessImageNameW", e))?;
    if length == 0 || length as usize >= path.len() {
        return Err(GamingError::Unsupported("Invalid process image length"));
    }
    let image = String::from_utf16(&path[..length as usize])
        .map_err(|_| GamingError::Unsupported("Invalid process image encoding"))?;
    if token != expected.creation_token
        || token == 0
        || image != expected.image_path
        || exit.dwHighDateTime != 0
        || exit.dwLowDateTime != 0
    {
        return Err(GamingError::Changed);
    }
    Ok(())
}
fn with_process<T>(
    expected: &ProcessIdentity,
    writing: bool,
    operation: impl FnOnce(HANDLE) -> Result<T, GamingError>,
) -> Result<T, GamingError> {
    if expected.pid == 0 || expected.creation_token == 0 || expected.image_path.is_empty() {
        return Err(GamingError::InvalidValue);
    }
    let access = if writing {
        PROCESS_QUERY_INFORMATION | PROCESS_SET_INFORMATION
    } else {
        PROCESS_QUERY_INFORMATION
    };
    let guard = HandleGuard(
        unsafe { OpenProcess(access, false, expected.pid) }
            .map_err(|e| win("OpenProcess gaming", e))?,
    );
    verify_process(guard.0, expected)?;
    let value = operation(guard.0)?;
    verify_process(guard.0, expected)?;
    Ok(value)
}
fn power_state(handle: HANDLE) -> Result<PowerThrottling, GamingError> {
    let mut state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..Default::default()
    };
    unsafe {
        GetProcessInformation(
            handle,
            ProcessPowerThrottling,
            (&mut state as *mut PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(|error| {
        if error.code().0 as u32 & 0xffff == 87 {
            GamingError::Unsupported(
                "Windows cannot read this process power policy; exact restoration is unavailable",
            )
        } else {
            win("GetProcessInformation power policy", error)
        }
    })?;
    if state.Version != PROCESS_POWER_THROTTLING_CURRENT_VERSION {
        return Err(GamingError::Unsupported(
            "Unknown process power policy version",
        ));
    }
    Ok(PowerThrottling {
        version: state.Version,
        control_mask: state.ControlMask,
        state_mask: state.StateMask,
    })
}
fn owned(state: PowerThrottling, bit: u32) -> OwnedPolicy {
    OwnedPolicy {
        control: state.control_mask & bit,
        state: state.state_mask & bit,
    }
}
pub fn read_process_policy(
    process: &ProcessIdentity,
    bit: u32,
) -> Result<OwnedPolicy, GamingError> {
    if !matches!(bit, EXECUTION_SPEED | IGNORE_TIMER_RESOLUTION) {
        return Err(GamingError::InvalidValue);
    }
    with_process(process, false, |handle| {
        Ok(owned(power_state(handle)?, bit))
    })
}
pub fn write_process_policy(
    process: &ProcessIdentity,
    bit: u32,
    expected: OwnedPolicy,
    desired: OwnedPolicy,
) -> Result<(), GamingError> {
    with_process(process, true, |handle| {
        verify_process(handle, process)?;
        let current = power_state(handle)?;
        if owned(current, bit) != expected {
            return Err(GamingError::Changed);
        }
        let merged = merge_policy(current, bit, desired)?;
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: merged.version,
            ControlMask: merged.control_mask,
            StateMask: merged.state_mask,
        };
        unsafe {
            SetProcessInformation(
                handle,
                ProcessPowerThrottling,
                (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        }
        .map_err(|e| win("SetProcessInformation power policy", e))?;
        if power_state(handle)? != merged {
            return Err(GamingError::Changed);
        }
        Ok(())
    })
}

fn topology(handle: Option<HANDLE>) -> Result<Vec<CpuSet>, GamingError> {
    for _ in 0..3 {
        let mut length = 0;
        let initial = unsafe { GetSystemCpuSetInformation(None, 0, &mut length, handle, Some(0)) };
        if initial.as_bool() && length == 0 {
            return Ok(Vec::new());
        }
        if initial.as_bool() || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err(code(
                "GetSystemCpuSetInformation size",
                unsafe { GetLastError() }.0,
            ));
        }
        if length == 0 || length > 1024 * 1024 {
            return Err(GamingError::Unsupported(
                "CPU Set topology exceeds the bounded size",
            ));
        }
        let capacity = length;
        let mut aligned = vec![0u64; (length as usize).div_ceil(8)];
        let base = aligned.as_mut_ptr().cast::<u8>();
        let success = unsafe {
            GetSystemCpuSetInformation(Some(base.cast()), capacity, &mut length, handle, Some(0))
        };
        if !success.as_bool() {
            if unsafe { GetLastError() } == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            return Err(code(
                "GetSystemCpuSetInformation",
                unsafe { GetLastError() }.0,
            ));
        }
        if length > capacity {
            return Err(GamingError::Changed);
        }
        let mut offset = 0usize;
        let mut result = Vec::new();
        while offset < length as usize {
            if length as usize - offset < 8 {
                return Err(GamingError::Unsupported("Truncated CPU Set topology"));
            }
            let record_length =
                unsafe { ptr::read_unaligned(base.add(offset).cast::<u32>()) } as usize;
            if record_length < 8 || record_length > length as usize - offset {
                return Err(GamingError::Unsupported("Invalid CPU Set record length"));
            }
            let kind = unsafe { ptr::read_unaligned(base.add(offset + 4).cast::<i32>()) };
            if kind == CpuSetInformation.0 {
                if record_length < size_of::<SYSTEM_CPU_SET_INFORMATION>() {
                    return Err(GamingError::Unsupported("Truncated CPU Set record"));
                }
                let record = unsafe {
                    ptr::read_unaligned(base.add(offset).cast::<SYSTEM_CPU_SET_INFORMATION>())
                };
                let cpu = unsafe { record.Anonymous.CpuSet };
                let flags = unsafe { cpu.Anonymous1.AllFlags };
                result.push(CpuSet {
                    id: cpu.Id,
                    group: cpu.Group,
                    logical_processor: cpu.LogicalProcessorIndex,
                    core: cpu.CoreIndex,
                    efficiency_class: cpu.EfficiencyClass,
                    parked: flags & 1 != 0,
                    allocated: flags & 2 != 0,
                    allocated_to_target: flags & 4 != 0,
                });
                if result.len() > 16384 {
                    return Err(GamingError::Unsupported("Too many CPU Sets"));
                }
            }
            offset += record_length;
        }
        return Ok(result);
    }
    Err(GamingError::Changed)
}
pub fn cpu_topology(process: Option<&ProcessIdentity>) -> Result<Vec<CpuSet>, GamingError> {
    match process {
        Some(process) => with_process(process, false, |handle| topology(Some(handle))),
        None => topology(None),
    }
}
fn default_sets(handle: HANDLE) -> Result<Vec<u32>, GamingError> {
    for _ in 0..3 {
        let mut count = 0;
        let initial = unsafe { GetProcessDefaultCpuSets(handle, None, &mut count) };
        if !initial.as_bool() && unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err(code(
                "GetProcessDefaultCpuSets size",
                unsafe { GetLastError() }.0,
            ));
        }
        if count > 16384 {
            return Err(GamingError::Unsupported("Too many process CPU Set IDs"));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut ids = vec![0u32; count as usize];
        let success = unsafe { GetProcessDefaultCpuSets(handle, Some(&mut ids), &mut count) };
        if !success.as_bool() {
            if unsafe { GetLastError() } == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            return Err(code(
                "GetProcessDefaultCpuSets",
                unsafe { GetLastError() }.0,
            ));
        }
        if count as usize > ids.len() {
            return Err(GamingError::Changed);
        }
        ids.truncate(count as usize);
        return Ok(ids);
    }
    Err(GamingError::Changed)
}
pub fn read_cpu_sets(process: &ProcessIdentity) -> Result<Vec<u32>, GamingError> {
    with_process(process, false, default_sets)
}
pub fn write_cpu_sets(
    process: &ProcessIdentity,
    expected: &[u32],
    desired: &[u32],
) -> Result<(), GamingError> {
    with_process(process, true, |handle| {
        validate_cpu_set_selection(desired, &topology(Some(handle))?)?;
        verify_process(handle, process)?;
        if default_sets(handle)? != expected {
            return Err(GamingError::Changed);
        }
        if !unsafe {
            SetProcessDefaultCpuSets(
                handle,
                if desired.is_empty() {
                    None
                } else {
                    Some(desired)
                },
            )
        }
        .as_bool()
        {
            return Err(code(
                "SetProcessDefaultCpuSets",
                unsafe { GetLastError() }.0,
            ));
        }
        if default_sets(handle)? != desired {
            return Err(GamingError::Changed);
        }
        Ok(())
    })
}
fn priority(handle: HANDLE) -> Result<u32, GamingError> {
    let raw = unsafe { GetPriorityClass(handle) };
    if raw == 0 {
        Err(code("GetPriorityClass", unsafe { GetLastError() }.0))
    } else {
        Ok(raw)
    }
}
pub fn read_priority(process: &ProcessIdentity) -> Result<u32, GamingError> {
    with_process(process, false, priority)
}
pub fn write_priority(
    process: &ProcessIdentity,
    expected: u32,
    desired: u32,
) -> Result<(), GamingError> {
    // Realtime is representable only to restore an exact pre-existing baseline; configs exclude it.
    if ![0x40, 0x4000, 0x20, 0x8000, 0x80, 0x100].contains(&desired) {
        return Err(GamingError::InvalidValue);
    }
    with_process(process, true, |handle| {
        verify_process(handle, process)?;
        if priority(handle)? != expected {
            return Err(GamingError::Changed);
        }
        unsafe { SetPriorityClass(handle, PROCESS_CREATION_FLAGS(desired)) }
            .map_err(|e| win("SetPriorityClass", e))?;
        if priority(handle)? != desired {
            return Err(GamingError::Changed);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interactive_user_accepts_only_exact_or_os_linked_same_account_session() {
        use crate::tuning::TuningTokenIdentity;
        let shell = TuningTokenIdentity {
            sid: "S-1-5-21-1".into(),
            session_id: 2,
            authentication_id: 10,
        };
        let mut caller = shell.clone();
        assert!(caller_matches_shell(&caller, &shell, None));
        caller.authentication_id = 11;
        assert!(
            caller_matches_shell(&caller, &shell, Some(&shell)),
            "The OS-linked UAC token identifies the same desktop logon"
        );
        assert!(!caller_matches_shell(&caller, &shell, None));
        assert!(!caller_matches_shell(&caller, &shell, Some(&caller)));
        let mut foreign = caller.clone();
        foreign.sid = "S-1-5-21-2".into();
        assert!(!caller_matches_shell(&foreign, &shell, Some(&shell)));
        foreign = caller.clone();
        foreign.session_id = 3;
        assert!(!caller_matches_shell(&foreign, &shell, Some(&shell)));
        let mut unrelated_link = shell.clone();
        unrelated_link.authentication_id = 12;
        assert!(!caller_matches_shell(
            &caller,
            &shell,
            Some(&unrelated_link)
        ));
    }
    #[test]
    fn gaming_process_read_rejects_wrong_creation_token_and_image_without_writes() {
        let identity = OS::tuning_process_identity(std::process::id()).unwrap();
        let process = ProcessIdentity {
            pid: identity.pid(),
            creation_token: identity.creation_time(),
            image_path: identity.image_path().to_string(),
        };
        assert!(read_priority(&process).is_ok());
        let mut wrong_token = process.clone();
        wrong_token.creation_token ^= 1;
        assert_eq!(read_priority(&wrong_token), Err(GamingError::Changed));
        let mut wrong_image = process;
        wrong_image.image_path.push_str(".reused");
        assert_eq!(read_priority(&wrong_image), Err(GamingError::Changed));
    }
}
