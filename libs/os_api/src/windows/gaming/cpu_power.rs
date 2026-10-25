//! Conditional AC-only processor power overrides, including explicit parking policy.
pub(crate) mod parking;
use super::{active_scheme, admit_account, code, guid, success, verify_user, with_account};
use crate::{
    gaming::{GamingError, GuidValue, POWER_MODE_PERFORMANCE, PowerSource, UserIdentity},
    gaming_cpu_power::{
        CpuPowerKey, CpuPowerOptions, CpuPowerRequest, CpuPowerSetting,
        validate_requested_frequency, validate_scalar,
    },
};
use std::{
    io,
    mem::size_of,
    sync::{Mutex, MutexGuard},
};
use windows::{
    Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NOT_ALL_ASSIGNED, ERROR_NOT_SUPPORTED,
            ERROR_PATH_NOT_FOUND, GetLastError, HANDLE, LUID,
        },
        Security::{
            AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
            SE_PRIVILEGE_ENABLED, SE_RESTORE_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
            TOKEN_QUERY,
        },
        System::{
            Power::{PowerReadACValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex},
            Registry::{
                HKEY, REG_DWORD, REG_OPENED_EXISTING_KEY, REG_OPTION_BACKUP_RESTORE, REG_SAM_FLAGS,
                REG_VALUE_TYPE, RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW,
            },
            SystemServices::{
                GUID_PROCESSOR_CORE_PARKING_MAX_CORES, GUID_PROCESSOR_CORE_PARKING_MAX_CORES_1,
                GUID_PROCESSOR_CORE_PARKING_MIN_CORES, GUID_PROCESSOR_CORE_PARKING_MIN_CORES_1,
                GUID_PROCESSOR_FREQUENCY_LIMIT, GUID_PROCESSOR_FREQUENCY_LIMIT_1,
                GUID_PROCESSOR_PERF_BOOST_MODE, GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE,
                GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE_1,
                GUID_PROCESSOR_SETTINGS_SUBGROUP, GUID_PROCESSOR_THROTTLE_MAXIMUM,
                GUID_PROCESSOR_THROTTLE_MAXIMUM_1, GUID_PROCESSOR_THROTTLE_MINIMUM,
                GUID_PROCESSOR_THROTTLE_MINIMUM_1,
            },
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
    core::{GUID, PCWSTR, w},
};
use winreg::{
    RegKey,
    enums::{HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_READ},
};

const SCHEMES: &str = r"SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes";
const SETTINGS: &str = r"SYSTEM\CurrentControlSet\Control\Power\PowerSettings";

static RESTORE_PRIVILEGE_SCOPE: Mutex<()> = Mutex::new(());

/// Temporarily enables only the caller's existing restore privilege. No ACL changes.
struct RestorePrivilegeGuard {
    token: super::HandleGuard,
    previous: TOKEN_PRIVILEGES,
    active: bool,
    _scope: MutexGuard<'static, ()>,
}

fn privilege_result(value: u32) -> Result<(), GamingError> {
    if value == ERROR_NOT_ALL_ASSIGNED.0 {
        Err(GamingError::Denied {
            operation: "enable CPU AC restoration privilege",
            code: value,
        })
    } else {
        success("adjust CPU AC restoration privilege", value)
    }
}

impl RestorePrivilegeGuard {
    fn enable() -> Result<Self, GamingError> {
        let scope = RESTORE_PRIVILEGE_SCOPE.lock().map_err(|_| {
            GamingError::Unsupported("CPU restoration privilege scope is unavailable")
        })?;
        let mut token = HANDLE::default();
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
                &mut token,
            )
        }
        .map_err(|error| super::win("open CPU restoration token", error))?;
        let token = super::HandleGuard(token);
        let mut luid = LUID::default();
        unsafe { LookupPrivilegeValueW(PCWSTR::null(), SE_RESTORE_NAME, &mut luid) }
            .map_err(|error| super::win("lookup CPU restoration privilege", error))?;
        let requested = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut previous = TOKEN_PRIVILEGES::default();
        let mut length = 0;
        unsafe {
            AdjustTokenPrivileges(
                token.0,
                false,
                Some(&requested),
                size_of::<TOKEN_PRIVILEGES>() as u32,
                Some(&mut previous),
                Some(&mut length),
            )
        }
        .map_err(|error| super::win("enable CPU restoration privilege", error))?;
        // TRUE is also returned for ERROR_NOT_ALL_ASSIGNED; inspect it immediately.
        let adjusted = unsafe { GetLastError() }.0;
        let mut guard = Self {
            token,
            previous,
            active: true,
            _scope: scope,
        };
        if let Err(error) = privilege_result(adjusted) {
            guard.restore()?;
            return Err(error);
        }
        Ok(guard)
    }

    fn restore(&mut self) -> Result<(), GamingError> {
        if self.active {
            unsafe {
                AdjustTokenPrivileges(self.token.0, false, Some(&self.previous), 0, None, None)
            }
            .map_err(|error| super::win("restore CPU token privileges", error))?;
            privilege_result(unsafe { GetLastError() }.0)?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for RestorePrivilegeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

/// Reopens the exact retained object, never a path that could create missing keys.
fn open_restore_handle(existing: &RegKey) -> Result<RegKey, GamingError> {
    let mut raw = HKEY::default();
    let mut disposition = REG_OPENED_EXISTING_KEY;
    success(
        "open exact CPU AC restoration handle",
        unsafe {
            RegCreateKeyExW(
                HKEY(existing.raw_handle()),
                w!(""),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_BACKUP_RESTORE,
                REG_SAM_FLAGS(0),
                None,
                &mut raw,
                Some(&mut disposition),
            )
        }
        .0,
    )?;
    let key = RegKey::predef(raw.0);
    if disposition != REG_OPENED_EXISTING_KEY {
        return Err(GamingError::Unsupported(
            "CPU restoration did not open the existing key",
        ));
    }
    Ok(key)
}

fn setting_guid(setting: CpuPowerSetting) -> GUID {
    match setting {
        CpuPowerSetting::Frequency => GUID_PROCESSOR_FREQUENCY_LIMIT,
        CpuPowerSetting::Frequency1 => GUID_PROCESSOR_FREQUENCY_LIMIT_1,
        CpuPowerSetting::Maximum => GUID_PROCESSOR_THROTTLE_MAXIMUM,
        CpuPowerSetting::Maximum1 => GUID_PROCESSOR_THROTTLE_MAXIMUM_1,
        CpuPowerSetting::Minimum => GUID_PROCESSOR_THROTTLE_MINIMUM,
        CpuPowerSetting::Minimum1 => GUID_PROCESSOR_THROTTLE_MINIMUM_1,
        CpuPowerSetting::EnergyPreference => GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE,
        CpuPowerSetting::EnergyPreference1 => GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE_1,
        CpuPowerSetting::Boost => GUID_PROCESSOR_PERF_BOOST_MODE,
        CpuPowerSetting::ParkingMinimum => GUID_PROCESSOR_CORE_PARKING_MIN_CORES,
        CpuPowerSetting::ParkingMinimum1 => GUID_PROCESSOR_CORE_PARKING_MIN_CORES_1,
        CpuPowerSetting::ParkingMaximum => GUID_PROCESSOR_CORE_PARKING_MAX_CORES,
        CpuPowerSetting::ParkingMaximum1 => GUID_PROCESSOR_CORE_PARKING_MAX_CORES_1,
    }
}

fn class_one(setting: CpuPowerSetting) -> bool {
    matches!(
        setting,
        CpuPowerSetting::Frequency1
            | CpuPowerSetting::Maximum1
            | CpuPowerSetting::Minimum1
            | CpuPowerSetting::EnergyPreference1
            | CpuPowerSetting::ParkingMinimum1
            | CpuPowerSetting::ParkingMaximum1
    )
}

fn format_guid(value: GUID) -> String {
    let value = value.to_u128();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        value >> 96,
        (value >> 80) & 0xffff,
        (value >> 64) & 0xffff,
        (value >> 48) & 0xffff,
        value & 0xffff_ffff_ffff
    )
}

fn setting_path(setting: CpuPowerSetting) -> String {
    format!(
        "{}\\{}",
        format_guid(GUID_PROCESSOR_SETTINGS_SUBGROUP),
        format_guid(setting_guid(setting))
    )
}

fn io_error(operation: &'static str, error: io::Error) -> GamingError {
    code(operation, error.raw_os_error().unwrap_or(1) as u32)
}

fn open_scheme(scheme: GuidValue) -> Result<RegKey, GamingError> {
    if scheme == GuidValue([0; 16]) {
        return Err(GamingError::InvalidValue);
    }
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            format!("{SCHEMES}\\{}", format_guid(guid(scheme))),
            KEY_READ,
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                GamingError::Unsupported("The captured CPU power scheme no longer exists")
            } else {
                io_error("open captured CPU power scheme", error)
            }
        })
}

fn metadata_available(setting: CpuPowerSetting) -> Result<bool, GamingError> {
    match RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(format!("{SETTINGS}\\{}", setting_path(setting)), KEY_READ)
    {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("read CPU power setting metadata", error)),
    }
}

/// A four-byte query prevents foreign types or oversized values entering recovery.
fn query_override(key: &RegKey) -> Result<Option<u32>, GamingError> {
    let mut value_type = REG_VALUE_TYPE(0);
    let mut value = [0u8; 4];
    let mut length = value.len() as u32;
    let result = unsafe {
        RegQueryValueExW(
            HKEY(key.raw_handle()),
            w!("ACSettingIndex"),
            None,
            Some(&mut value_type),
            Some(value.as_mut_ptr()),
            Some(&mut length),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if result == ERROR_MORE_DATA {
        return Err(GamingError::Unsupported(
            "CPU AC override is not a bounded DWORD",
        ));
    }
    success("read CPU AC override", result.0)?;
    decode_override(value_type, length, value).map(Some)
}

fn decode_override(
    value_type: REG_VALUE_TYPE,
    length: u32,
    value: [u8; 4],
) -> Result<u32, GamingError> {
    if value_type != REG_DWORD || length != 4 {
        return Err(GamingError::Unsupported(
            "CPU AC override has an invalid registry type or size",
        ));
    }
    Ok(u32::from_le_bytes(value))
}

fn read_override(key: CpuPowerKey) -> Result<Option<u32>, GamingError> {
    let scheme = open_scheme(key.scheme)?;
    let value = match scheme.open_subkey_with_flags(setting_path(key.setting), KEY_QUERY_VALUE) {
        Ok(setting) => query_override(&setting)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error("open CPU AC override", error)),
    };
    // A vanished captured scheme must never be reinterpreted as an absent value.
    open_scheme(key.scheme)?;
    validate_scalar(key.setting, value)?;
    Ok(value)
}

fn effective_value(key: CpuPowerKey) -> Result<u32, GamingError> {
    let mut value = 0;
    let result = unsafe {
        PowerReadACValueIndex(
            None,
            Some(&guid(key.scheme)),
            Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
            Some(&setting_guid(key.setting)),
            &mut value,
        )
    };
    success("PowerReadACValueIndex CPU", result.0)?;
    validate_scalar(key.setting, Some(value))?;
    Ok(value)
}

fn optional_unavailable(error: &GamingError) -> bool {
    matches!(
        error,
        GamingError::Native { code, .. }
            if [ERROR_FILE_NOT_FOUND.0, ERROR_PATH_NOT_FOUND.0, ERROR_NOT_SUPPORTED.0].contains(code)
    )
}

fn requested_value(
    setting: CpuPowerSetting,
    current: u32,
    options: &CpuPowerOptions,
) -> Option<u32> {
    let desired = match setting {
        // Class 1 is the higher-performance class (P); class 0 is E on a two-class CPU.
        // Legacy shared limits retain their original meaning for both classes.
        CpuPowerSetting::Frequency => options.efficient_mhz.unwrap_or(options.performance_mhz),
        CpuPowerSetting::Frequency1 => options.performance_mhz,
        CpuPowerSetting::Maximum | CpuPowerSetting::Maximum1 => 100,
        CpuPowerSetting::Minimum | CpuPowerSetting::Minimum1 if options.keep_high => 100,
        CpuPowerSetting::Minimum | CpuPowerSetting::Minimum1 if options.allow_parking.is_some() => {
            0
        }
        CpuPowerSetting::Minimum | CpuPowerSetting::Minimum1 => return None,
        CpuPowerSetting::ParkingMinimum | CpuPowerSetting::ParkingMinimum1 => {
            match options.allow_parking? {
                true => 0,
                false => 100,
            }
        }
        CpuPowerSetting::ParkingMaximum | CpuPowerSetting::ParkingMaximum1 => {
            options.allow_parking?;
            100
        }
        CpuPowerSetting::EnergyPreference | CpuPowerSetting::EnergyPreference1 => 0,
        CpuPowerSetting::Boost if current == 0 => 1,
        CpuPowerSetting::Boost => return None,
    };
    (current != desired).then_some(desired)
}

fn has_two_core_classes(cpus: &[crate::gaming::CpuSet]) -> bool {
    cpus.iter()
        .map(|cpu| cpu.efficiency_class)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == 2
}

/// Reuses the existing bounded CPU Set read; no affinity or scheduling changes.
pub fn hybrid_cores(user: &UserIdentity) -> Result<bool, GamingError> {
    with_account(user, || {
        super::cpu_topology(None).map(|cpus| has_two_core_classes(&cpus))
    })
}

/// Reads current effective values but emits only changes. It never writes or selects a plan.
pub fn prepare_requests(
    user: &UserIdentity,
    options: &CpuPowerOptions,
) -> Result<Vec<CpuPowerRequest>, GamingError> {
    validate_requested_frequency(options.performance_mhz)?;
    if let Some(mhz) = options.efficient_mhz {
        validate_requested_frequency(mhz)?;
        if !hybrid_cores(user)? {
            return Err(GamingError::Unsupported(
                "Separate P/E limits require two verified CPU classes",
            ));
        }
    }
    with_account(user, || {
        let base = active_scheme()?;
        open_scheme(base)?;
        open_scheme(POWER_MODE_PERFORMANCE)?;
        let mut requests = Vec::new();
        for (index, scheme) in [base, POWER_MODE_PERFORMANCE].into_iter().enumerate() {
            if index == 1 && scheme == base {
                continue;
            }
            for setting in [
                CpuPowerSetting::Frequency,
                CpuPowerSetting::Frequency1,
                CpuPowerSetting::Maximum,
                CpuPowerSetting::Maximum1,
                CpuPowerSetting::Minimum,
                CpuPowerSetting::Minimum1,
                CpuPowerSetting::EnergyPreference,
                CpuPowerSetting::EnergyPreference1,
                CpuPowerSetting::Boost,
                CpuPowerSetting::ParkingMinimum,
                CpuPowerSetting::ParkingMinimum1,
                CpuPowerSetting::ParkingMaximum,
                CpuPowerSetting::ParkingMaximum1,
            ] {
                if !options.keep_high
                    && options.allow_parking.is_none()
                    && matches!(
                        setting,
                        CpuPowerSetting::Minimum | CpuPowerSetting::Minimum1
                    )
                {
                    continue;
                }
                if options.allow_parking.is_none()
                    && matches!(
                        setting,
                        CpuPowerSetting::ParkingMinimum
                            | CpuPowerSetting::ParkingMinimum1
                            | CpuPowerSetting::ParkingMaximum
                            | CpuPowerSetting::ParkingMaximum1
                    )
                {
                    continue;
                }
                if !metadata_available(setting)? {
                    if class_one(setting) && options.efficient_mhz.is_none() {
                        continue;
                    }
                    return Err(GamingError::Unsupported(
                        "This CPU power setting is unavailable",
                    ));
                }
                let key = CpuPowerKey { scheme, setting };
                read_override(key)?;
                let current = match effective_value(key) {
                    Ok(value) => value,
                    Err(error)
                        if class_one(setting)
                            && optional_unavailable(&error)
                            && options.efficient_mhz.is_none() =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if let Some(desired) = requested_value(setting, current, options) {
                    requests.push(CpuPowerRequest {
                        key,
                        value: desired,
                    });
                }
            }
        }
        if active_scheme()? != base {
            return Err(GamingError::Changed);
        }
        Ok(requests)
    })
}

/// Returns the captured scheme's exact override; absence remains distinct from zero.
pub fn read_value(user: &UserIdentity, key: CpuPowerKey) -> Result<Option<u32>, GamingError> {
    with_account(user, || read_override(key))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshTarget {
    Base,
    Overlay,
}

fn refresh_target(scheme: GuidValue, base: GuidValue, overlay: GuidValue) -> Option<RefreshTarget> {
    if scheme == base {
        Some(RefreshTarget::Base)
    } else if [POWER_MODE_PERFORMANCE, crate::gaming::POWER_MODE_EFFICIENCY].contains(&scheme)
        && scheme == overlay
    {
        Some(RefreshTarget::Overlay)
    } else {
        None
    }
}

fn refresh_scheme(user: &UserIdentity, scheme: GuidValue) -> Result<(), GamingError> {
    let base = active_scheme()?;
    let overlay = if [POWER_MODE_PERFORMANCE, crate::gaming::POWER_MODE_EFFICIENCY]
        .contains(&scheme)
        && scheme != base
    {
        super::read_power_mode(user, PowerSource::Ac)?
    } else {
        GuidValue([0; 16])
    };
    match refresh_target(scheme, base, overlay) {
        Some(RefreshTarget::Base) => {
            verify_user(user)?;
            if active_scheme()? != scheme {
                return Err(GamingError::Changed);
            }
            success(
                "PowerSetActiveScheme CPU refresh",
                unsafe { PowerSetActiveScheme(None, Some(&guid(scheme))) }.0,
            )?;
            if active_scheme()? != scheme {
                return Err(GamingError::Changed);
            }
        }
        Some(RefreshTarget::Overlay) => {
            super::write_power_mode(user, PowerSource::Ac, scheme, scheme)?;
        }
        None => {}
    }
    Ok(())
}

fn conditional_update(
    expected: Option<u32>,
    desired: Option<u32>,
    mut read: impl FnMut() -> Result<Option<u32>, GamingError>,
    write: impl FnOnce(Option<u32>) -> Result<(), GamingError>,
    refresh: impl FnOnce() -> Result<(), GamingError>,
) -> Result<(), GamingError> {
    if read()? != expected {
        return Err(GamingError::Changed);
    }
    if expected == desired {
        return Ok(());
    }
    write(desired)?;
    refresh()?;
    if read()? != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}

fn store_override(
    key: CpuPowerKey,
    expected: Option<u32>,
    desired: Option<u32>,
) -> Result<(), GamingError> {
    let scheme = open_scheme(key.scheme)?;
    if let Some(value) = desired {
        // An absent original needs value deletion on Undo. Prove that privilege
        // is available before the SDK can create its first AC DWORD.
        let mut privilege = if expected.is_none() {
            Some(RestorePrivilegeGuard::enable()?)
        } else {
            None
        };
        let written = (|| {
            if expected.is_none() {
                match scheme.open_subkey_with_flags(setting_path(key.setting), KEY_QUERY_VALUE) {
                    Ok(existing) => {
                        open_restore_handle(&existing)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(io_error("preflight CPU AC restoration", error)),
                }
            }
            if read_override(key)? != expected {
                return Err(GamingError::Changed);
            }
            success(
                "PowerWriteACValueIndex CPU",
                unsafe {
                    PowerWriteACValueIndex(
                        None,
                        &guid(key.scheme),
                        Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                        Some(&setting_guid(key.setting)),
                        value,
                    )
                }
                .0,
            )?;
            if effective_value(key)? != value {
                return Err(GamingError::Changed);
            }
            Ok(())
        })();
        if let Some(guard) = &mut privilege {
            guard.restore()?;
        }
        written?;
    } else {
        let setting = scheme
            .open_subkey_with_flags(setting_path(key.setting), KEY_QUERY_VALUE)
            .map_err(|error| io_error("open CPU AC override for restoration", error))?;
        let mut privilege = RestorePrivilegeGuard::enable()?;
        let writable = open_restore_handle(&setting);
        privilege.restore()?;
        let writable = writable?;
        if query_override(&setting)? != expected {
            return Err(GamingError::Changed);
        }
        // Delete only our exact AC DWORD; retain the setting key, DC and all other fields.
        success(
            "restore absent CPU AC override",
            unsafe { RegDeleteValueW(HKEY(writable.raw_handle()), w!("ACSettingIndex")) }.0,
        )?;
    }
    Ok(())
}

/// A failed refresh/readback remains an error after a write so recovery retains evidence.
pub fn confirm_value(
    user: &UserIdentity,
    key: CpuPowerKey,
    expected: Option<u32>,
) -> Result<(), GamingError> {
    validate_scalar(key.setting, expected)?;
    let current = admit_account(user)?;
    if read_override(key)? != expected {
        return Err(GamingError::Changed);
    }
    verify_user(&current)?;
    refresh_scheme(&current, key.scheme)?;
    if read_override(key)? != expected {
        return Err(GamingError::Changed);
    }
    verify_user(&current)
}

pub fn write_value(
    user: &UserIdentity,
    key: CpuPowerKey,
    expected: Option<u32>,
    desired: Option<u32>,
) -> Result<(), GamingError> {
    validate_scalar(key.setting, expected)?;
    validate_scalar(key.setting, desired)?;
    let current = admit_account(user)?;
    conditional_update(
        expected,
        desired,
        || read_override(key),
        |desired| {
            verify_user(&current)?;
            store_override(key, expected, desired)
        },
        || refresh_scheme(&current, key.scheme),
    )?;
    verify_user(&current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gaming::GamingError;
    use std::cell::Cell;

    #[test]
    fn parking_refreshes_only_the_captured_current_efficiency_overlay() {
        let base = GuidValue([5; 16]);
        let efficiency = crate::gaming::POWER_MODE_EFFICIENCY;
        assert_eq!(
            refresh_target(efficiency, base, efficiency),
            Some(RefreshTarget::Overlay)
        );
        assert_eq!(
            refresh_target(efficiency, base, POWER_MODE_PERFORMANCE),
            None
        );
    }

    #[test]
    fn separate_hybrid_limits_use_p_class_one_and_e_class_zero() {
        let options = CpuPowerOptions {
            performance_mhz: 4500,
            efficient_mhz: Some(2500),
            keep_high: true,
            allow_parking: Some(false),
        };
        assert_eq!(
            requested_value(CpuPowerSetting::Frequency1, 0, &options),
            Some(4500)
        );
        assert_eq!(
            requested_value(CpuPowerSetting::Frequency, 0, &options),
            Some(2500)
        );
        for setting in [
            CpuPowerSetting::ParkingMinimum,
            CpuPowerSetting::ParkingMinimum1,
            CpuPowerSetting::ParkingMaximum,
            CpuPowerSetting::ParkingMaximum1,
        ] {
            assert_eq!(requested_value(setting, 50, &options), Some(100));
        }
        let allowed = CpuPowerOptions {
            keep_high: false,
            allow_parking: Some(true),
            ..options
        };
        for setting in [
            CpuPowerSetting::ParkingMinimum,
            CpuPowerSetting::ParkingMinimum1,
            CpuPowerSetting::Minimum,
            CpuPowerSetting::Minimum1,
        ] {
            assert_eq!(requested_value(setting, 100, &allowed), Some(0));
        }
        let legacy = CpuPowerOptions {
            performance_mhz: 4500,
            ..Default::default()
        };
        assert_eq!(
            requested_value(CpuPowerSetting::Frequency1, 0, &legacy),
            Some(4500)
        );
        assert_eq!(
            requested_value(CpuPowerSetting::Frequency, 0, &legacy),
            Some(4500)
        );
        assert_eq!(
            requested_value(CpuPowerSetting::ParkingMinimum, 100, &legacy),
            None
        );
        assert_eq!(
            requested_value(CpuPowerSetting::Minimum, 100, &legacy),
            None
        );
    }

    #[test]
    fn stored_override_requires_exact_dword_type_and_preserves_original_bytes() {
        use windows::Win32::System::Registry::REG_SZ;
        assert_eq!(
            decode_override(REG_DWORD, 4, 5401u32.to_le_bytes()),
            Ok(5401)
        );
        assert!(decode_override(REG_SZ, 4, 5401u32.to_le_bytes()).is_err());
        assert!(decode_override(REG_DWORD, 3, 5401u32.to_le_bytes()).is_err());
    }

    #[test]
    fn optional_support_does_not_hide_denial_or_unexpected_native_failures() {
        assert!(optional_unavailable(&GamingError::Native {
            operation: "read",
            code: ERROR_FILE_NOT_FOUND.0,
        }));
        assert!(!optional_unavailable(&GamingError::Denied {
            operation: "read",
            code: 5
        }));
        assert!(!optional_unavailable(&GamingError::Native {
            operation: "read",
            code: 87
        }));
        assert!(!optional_unavailable(&GamingError::Changed));
    }

    #[test]
    fn native_keys_resolve_to_sdk_whitelist_and_never_include_dc_or_other_settings() {
        assert_eq!(
            setting_path(CpuPowerSetting::Frequency),
            "54533251-82be-4824-96c1-47b60b740d00\\75b0ae3f-bce0-45a7-8c89-c9611c25e100"
        );
        assert_eq!(
            setting_guid(CpuPowerSetting::Boost),
            GUID_PROCESSOR_PERF_BOOST_MODE
        );
        assert_eq!(
            setting_guid(CpuPowerSetting::EnergyPreference1),
            GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE_1
        );
        assert!(class_one(CpuPowerSetting::Maximum1));
        assert!(!class_one(CpuPowerSetting::Maximum));
    }

    #[test]
    fn preparation_omits_defaults_and_enabled_boost_without_resetting_its_mode() {
        let requested_value = |setting, current, maximum| {
            super::requested_value(
                setting,
                current,
                &CpuPowerOptions {
                    performance_mhz: maximum,
                    keep_high: true,
                    ..Default::default()
                },
            )
        };
        assert_eq!(requested_value(CpuPowerSetting::Frequency, 0, 0), None);
        assert_eq!(
            requested_value(CpuPowerSetting::Frequency, 5401, 5400),
            Some(5400)
        );
        assert_eq!(requested_value(CpuPowerSetting::Maximum, 100, 0), None);
        assert_eq!(
            requested_value(CpuPowerSetting::EnergyPreference, 0, 0),
            None
        );
        assert_eq!(
            requested_value(CpuPowerSetting::EnergyPreference1, 50, 0),
            Some(0)
        );
        assert_eq!(requested_value(CpuPowerSetting::Minimum, 5, 0), Some(100));
        assert_eq!(requested_value(CpuPowerSetting::Boost, 0, 0), Some(1));
        for mode in 1..=6 {
            assert_eq!(requested_value(CpuPowerSetting::Boost, mode, 0), None);
        }
    }

    #[test]
    fn nullable_guard_restores_absence_and_preserves_drift() {
        let value = Cell::new(Some(5400));
        let stores = Cell::new(0);
        conditional_update(
            Some(5400),
            None,
            || Ok(value.get()),
            |desired| {
                stores.set(stores.get() + 1);
                value.set(desired);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(value.get(), None);
        assert_eq!(stores.get(), 1);
        assert_eq!(
            conditional_update(
                Some(5400),
                Some(5500),
                || Ok(Some(5401)),
                |_| {
                    stores.set(stores.get() + 1);
                    Ok(())
                },
                || Ok(()),
            ),
            Err(GamingError::Changed)
        );
        assert_eq!(stores.get(), 1);
    }

    #[test]
    fn absent_and_zero_are_distinct_and_noop_does_not_refresh() {
        let value = Cell::new(None);
        conditional_update(
            None,
            Some(0),
            || Ok(value.get()),
            |desired| {
                value.set(desired);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(value.get(), Some(0));
        conditional_update(
            Some(0),
            Some(0),
            || Ok(value.get()),
            |_| panic!("no-op must not write"),
            || panic!("no-op must not refresh"),
        )
        .unwrap();
    }

    #[test]
    fn refresh_and_readback_failures_after_write_remain_errors() {
        let value = Cell::new(Some(1));
        let failure = GamingError::Native {
            operation: "refresh",
            code: 1,
        };
        assert_eq!(
            conditional_update(
                Some(1),
                Some(0),
                || Ok(value.get()),
                |desired| {
                    value.set(desired);
                    Ok(())
                },
                || Err(failure.clone()),
            ),
            Err(failure)
        );
        assert_eq!(value.get(), Some(0));
        let reads = Cell::new(0);
        assert_eq!(
            conditional_update(
                Some(1),
                Some(0),
                || {
                    reads.set(reads.get() + 1);
                    if reads.get() == 1 {
                        Ok(Some(1))
                    } else {
                        Err(GamingError::Changed)
                    }
                },
                |_| Ok(()),
                || Ok(()),
            ),
            Err(GamingError::Changed)
        );
    }

    #[test]
    fn refresh_selection_never_activates_another_saved_scheme() {
        use crate::gaming::{GuidValue, POWER_MODE_PERFORMANCE};
        let base = GuidValue(0x381b4222_f694_41f0_9685_ff5bb260df2eu128.to_be_bytes());
        let another = GuidValue(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635cu128.to_be_bytes());
        assert_eq!(
            refresh_target(base, base, POWER_MODE_PERFORMANCE),
            Some(RefreshTarget::Base)
        );
        assert_eq!(
            refresh_target(POWER_MODE_PERFORMANCE, base, POWER_MODE_PERFORMANCE),
            Some(RefreshTarget::Overlay)
        );
        assert_eq!(refresh_target(another, base, POWER_MODE_PERFORMANCE), None);
        assert_eq!(
            refresh_target(POWER_MODE_PERFORMANCE, base, GuidValue([0; 16])),
            None
        );
    }
}
