//! Conditional per-user preference writes and documented GDI refresh changes.
//! No policy, Game Bar disablement, services or driver-specific settings.
use super::gaming::admit_account;
use crate::{
    gaming::{self, GamingError, UserIdentity},
    windows_tweaks::*,
};
use std::{io, mem::size_of, path::Path};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA},
        Graphics::Gdi::*,
        System::Registry::{HKEY, REG_VALUE_TYPE, RegQueryValueExW},
    },
    core::PCWSTR,
};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_SZ},
};

const GAME_BAR: &str = r"Software\Microsoft\GameBar";
const CAPTURE: &str = r"Software\Microsoft\Windows\CurrentVersion\GameDVR";
const GPU: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
const GLOBAL: &str = "DirectXUserGlobalSettings";

fn io_error(operation: &'static str, error: io::Error) -> GamingError {
    let code = error.raw_os_error().unwrap_or(1) as u32;
    if code == 5 {
        GamingError::Denied { operation, code }
    } else {
        GamingError::Native { operation, code }
    }
}
fn verify_user(user: &UserIdentity) -> Result<(), GamingError> {
    gaming::verified_session_user(user, gaming::interactive_user()).map(|_| ())
}
fn read_raw(path: &str, name: &str) -> Result<Option<winreg::RegValue<'static>>, GamingError> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("open preference", error)),
    };
    let name = wide(name);
    let mut bytes = vec![0u8; 32770];
    let mut length = bytes.len() as u32;
    let mut value_type = REG_VALUE_TYPE(0);
    let result = unsafe {
        RegQueryValueExW(
            HKEY(key.raw_handle()),
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(bytes.as_mut_ptr()),
            Some(&mut length),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if result == ERROR_MORE_DATA || length as usize > bytes.len() {
        return Err(GamingError::Unsupported(
            "Stored preference exceeds the bounded read limit",
        ));
    }
    if result.0 != 0 {
        return Err(io_error(
            "read preference",
            io::Error::from_raw_os_error(result.0 as i32),
        ));
    }
    bytes.truncate(length as usize);
    let vtype = match value_type.0 {
        1 => REG_SZ,
        4 => REG_DWORD,
        _ => {
            return Err(GamingError::Unsupported(
                "Stored preference has an unrecognized registry type",
            ));
        }
    };
    Ok(Some(winreg::RegValue {
        bytes: bytes.into(),
        vtype,
    }))
}
fn store(
    path: &str,
    name: &str,
    desired: Option<&winreg::RegValue<'_>>,
) -> Result<(), GamingError> {
    if let Some(value) = desired {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(path, KEY_READ | KEY_WRITE)
            .map_err(|error| io_error("open writable preference", error))?;
        key.set_raw_value(name, value)
            .map_err(|error| io_error("write preference", error))
    } else {
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(path, KEY_READ | KEY_WRITE)
        {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error("open writable preference", error)),
        };
        match key.delete_value(name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("restore absent preference", error)),
        }
    }
}
fn dword(raw: Option<&winreg::RegValue<'_>>) -> Result<StoredDword, GamingError> {
    match raw {
        None => Ok(StoredDword::Absent),
        Some(value) if value.vtype == REG_DWORD && value.bytes.len() == 4 => {
            let value = u32::from_le_bytes(
                value
                    .bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| GamingError::InvalidValue)?,
            );
            if value <= 1 {
                Ok(StoredDword::Value(value))
            } else {
                Err(GamingError::Unsupported("Unrecognized stored toggle value"))
            }
        }
        _ => Err(GamingError::Unsupported(
            "Stored toggle has an unrecognized registry type or size",
        )),
    }
}
fn read_toggle(user: &UserIdentity, path: &str, name: &str) -> Result<StoredDword, GamingError> {
    let current = admit_account(user)?;
    let result = dword(read_raw(path, name)?.as_ref())?;
    verify_user(&current)?;
    Ok(result)
}
fn write_toggle(
    user: &UserIdentity,
    path: &str,
    name: &str,
    expected: StoredDword,
    desired: StoredDword,
) -> Result<(), GamingError> {
    if matches!(desired, StoredDword::Value(value) if value > 1) {
        return Err(GamingError::InvalidValue);
    }
    let current = admit_account(user)?;
    let before = read_raw(path, name)?;
    if dword(before.as_ref())? != expected {
        return Err(GamingError::Changed);
    }
    let raw = desired.value().map(|value| winreg::RegValue {
        vtype: REG_DWORD,
        bytes: value.to_le_bytes().to_vec().into(),
    });
    verify_user(&current)?;
    if read_raw(path, name)? != before {
        return Err(GamingError::Changed);
    }
    store(path, name, raw.as_ref())?;
    let actual = read_toggle(&current, path, name)?;
    verify_user(&current)?;
    if actual != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}
/// Microsoft Windows 11 settings reference identifies this single toggle.
pub fn read_game_mode(user: &UserIdentity) -> Result<StoredDword, GamingError> {
    read_toggle(user, GAME_BAR, "AutoGameModeEnabled")
}
pub fn write_game_mode(
    user: &UserIdentity,
    expected: StoredDword,
    desired: StoredDword,
) -> Result<(), GamingError> {
    write_toggle(user, GAME_BAR, "AutoGameModeEnabled", expected, desired)
}
/// Only the historical buffer preference. No AppCaptureEnabled/GameDVR_Enabled/policy writes.
pub fn read_background_capture(user: &UserIdentity) -> Result<StoredDword, GamingError> {
    read_toggle(user, CAPTURE, "HistoricalCaptureEnabled")
}
pub fn write_background_capture(
    user: &UserIdentity,
    expected: StoredDword,
    desired: StoredDword,
) -> Result<(), GamingError> {
    write_toggle(user, CAPTURE, "HistoricalCaptureEnabled", expected, desired)
}

fn string_value(raw: Option<&winreg::RegValue<'_>>) -> Result<Option<String>, GamingError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.vtype != REG_SZ || raw.bytes.len() < 2 || !raw.bytes.len().is_multiple_of(2) {
        return Err(GamingError::Unsupported(
            "Stored graphics preference is not a bounded REG_SZ",
        ));
    }
    let mut wide: Vec<u16> = raw
        .bytes
        .as_ref()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect();
    if wide.pop() != Some(0) || wide.contains(&0) {
        return Err(GamingError::Unsupported(
            "Stored graphics preference has invalid termination",
        ));
    }
    String::from_utf16(&wide)
        .map(Some)
        .map_err(|_| GamingError::Unsupported("Stored graphics preference contains invalid UTF-16"))
}
fn string_raw(value: &str) -> winreg::RegValue<'static> {
    winreg::RegValue {
        vtype: REG_SZ,
        bytes: value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
            .into(),
    }
}
fn checked_field(value: Option<&str>, field: &str) -> Result<Option<String>, GamingError> {
    let result = preference_field(value, field)?;
    if result.as_deref().is_some_and(|value| {
        !matches!(
            (field, value),
            ("SwapEffectUpgradeEnable", "0" | "1") | ("GpuPreference", "0" | "1" | "2")
        )
    }) {
        return Err(GamingError::Unsupported(
            "Stored owned graphics field has an unrecognized value",
        ));
    }
    Ok(result)
}
fn read_field(user: &UserIdentity, name: &str, field: &str) -> Result<Option<String>, GamingError> {
    let current = admit_account(user)?;
    let raw = read_raw(GPU, name)?;
    let value = string_value(raw.as_ref())?;
    let result = checked_field(value.as_deref(), field)?;
    verify_user(&current)?;
    Ok(result)
}
fn write_field(
    user: &UserIdentity,
    name: &str,
    field: &str,
    expected: Option<&str>,
    desired: Option<&str>,
) -> Result<(), GamingError> {
    let current = admit_account(user)?;
    let before = read_raw(GPU, name)?;
    let value = string_value(before.as_ref())?;
    if checked_field(value.as_deref(), field)?.as_deref() != expected {
        return Err(GamingError::Changed);
    }
    if field == "SwapEffectUpgradeEnable" {
        windowed_restore_allowed(value.as_deref(), desired)?;
    }
    let merged = merge_preference(value.as_deref(), field, desired)?;
    let raw = merged.as_deref().map(string_raw);
    verify_user(&current)?;
    if read_raw(GPU, name)? != before {
        return Err(GamingError::Changed);
    }
    store(GPU, name, raw.as_ref())?;
    let actual = read_field(&current, name, field)?;
    verify_user(&current)?;
    if actual.as_deref() != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}
pub fn read_windowed_optimizations(user: &UserIdentity) -> Result<Option<String>, GamingError> {
    read_field(user, GLOBAL, "SwapEffectUpgradeEnable")
}
pub fn write_windowed_optimizations(
    user: &UserIdentity,
    expected: Option<&str>,
    desired: Option<&str>,
) -> Result<(), GamingError> {
    write_field(user, GLOBAL, "SwapEffectUpgradeEnable", expected, desired)
}

pub fn verified_executable(path: &str) -> Result<String, GamingError> {
    if path.len() > 32767 || path.contains('\0') || !Path::new(path).is_absolute() {
        return Err(GamingError::InvalidValue);
    }
    let resolved =
        std::fs::canonicalize(path).map_err(|error| io_error("verify executable path", error))?;
    if !resolved.is_file()
        || !resolved
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(GamingError::Unsupported(
            "Choose an existing executable file",
        ));
    }
    let resolved = resolved.to_str().ok_or(GamingError::Unsupported(
        "Executable path cannot be represented losslessly",
    ))?;
    Ok(preference_executable_path(resolved))
}
fn preference_executable_path(resolved: &str) -> String {
    if let Some(unc) = resolved.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        resolved
            .strip_prefix(r"\\?\")
            .unwrap_or(resolved)
            .to_owned()
    }
}
fn owned_gpu_path(path: &str) -> Result<String, GamingError> {
    use std::path::{Component, Prefix};
    let parsed = Path::new(path);
    if path.len() > 32767
        || path.contains('\0')
        || !parsed.is_absolute()
        || !parsed
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        || !matches!(parsed.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _)))
        || parsed
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(GamingError::InvalidValue);
    }
    // The controller already holds the exact qualified user/path and compares
    // the owned registry field. Uninstalling the EXE must not strand its Undo.
    Ok(path.to_owned())
}
pub fn qualify_gpu_target(path: &str) -> Result<GpuTarget, GamingError> {
    let user = gaming::interactive_user()?;
    let executable = verified_executable(path)?;
    verify_user(&user)?;
    Ok(GpuTarget {
        user_sid: user.sid,
        user_logon: 0,
        executable,
    })
}
pub fn read_gpu_preference(user: &UserIdentity, path: &str) -> Result<Option<String>, GamingError> {
    let path = owned_gpu_path(path)?;
    read_field(user, &path, "GpuPreference")
}
pub fn write_gpu_preference(
    user: &UserIdentity,
    path: &str,
    expected: Option<&str>,
    desired: Option<&str>,
) -> Result<(), GamingError> {
    let path = owned_gpu_path(path)?;
    write_field(user, &path, "GpuPreference", expected, desired)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn text(value: &[u16]) -> Result<String, GamingError> {
    let end = value
        .iter()
        .position(|value| *value == 0)
        .ok_or(GamingError::InvalidValue)?;
    String::from_utf16(&value[..end]).map_err(|_| GamingError::InvalidValue)
}
fn adapter(index: u32) -> Option<DISPLAY_DEVICEW> {
    let mut adapter = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut adapter, 0) }
        .as_bool()
        .then_some(adapter)
}
fn qualified_target(
    user: &UserIdentity,
    adapter: &DISPLAY_DEVICEW,
) -> Result<DisplayTarget, GamingError> {
    let mut monitor = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    if !unsafe { EnumDisplayDevicesW(PCWSTR(adapter.DeviceName.as_ptr()), 0, &mut monitor, 0) }
        .as_bool()
    {
        return Err(GamingError::Unsupported(
            "Cannot qualify the current display monitor",
        ));
    }
    let result = DisplayTarget {
        user_sid: user.sid.clone(),
        user_logon: user.logon,
        device_name: text(&adapter.DeviceName)?,
        adapter_id: text(&adapter.DeviceID)?,
        monitor_id: text(&monitor.DeviceID)?,
    };
    if result.adapter_id.is_empty() || result.monitor_id.is_empty() {
        return Err(GamingError::Unsupported(
            "Stable display device identities are unavailable",
        ));
    }
    Ok(result)
}
pub fn primary_display_target() -> Result<DisplayTarget, GamingError> {
    let user = gaming::interactive_user()?;
    for index in 0..64 {
        let Some(adapter) = adapter(index) else {
            break;
        };
        if adapter
            .StateFlags
            .contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP | DISPLAY_DEVICE_PRIMARY_DEVICE)
        {
            let target = qualified_target(&user, &adapter)?;
            verify_display(&target)?;
            return Ok(target);
        }
    }
    Err(GamingError::Unsupported(
        "No qualified primary desktop display",
    ))
}
fn verify_display(target: &DisplayTarget) -> Result<(), GamingError> {
    verify_user(&target.user())?;
    for index in 0..64 {
        let Some(adapter) = adapter(index) else {
            break;
        };
        if text(&adapter.DeviceName)? == target.device_name {
            if adapter
                .StateFlags
                .contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
                && qualified_target(&target.user(), &adapter)? == *target
            {
                return Ok(());
            }
            return Err(GamingError::Changed);
        }
    }
    Err(GamingError::Changed)
}
fn raw_mode(
    target: &DisplayTarget,
    index: ENUM_DISPLAY_SETTINGS_MODE,
) -> Result<DEVMODEW, GamingError> {
    let name = wide(&target.device_name);
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    if !unsafe {
        EnumDisplaySettingsExW(
            PCWSTR(name.as_ptr()),
            index,
            &mut mode,
            ENUM_DISPLAY_SETTINGS_FLAGS(0),
        )
    }
    .as_bool()
    {
        return Err(GamingError::Unsupported(
            "The requested display mode is unavailable",
        ));
    }
    Ok(mode)
}
fn mode_value(mode: &DEVMODEW) -> DisplayMode {
    DisplayMode {
        width: mode.dmPelsWidth,
        height: mode.dmPelsHeight,
        bits_per_pixel: mode.dmBitsPerPel,
        frequency: mode.dmDisplayFrequency,
        flags: unsafe { mode.Anonymous2.dmDisplayFlags },
        orientation: unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation.0 },
        fixed_output: unsafe { mode.Anonymous1.Anonymous2.dmDisplayFixedOutput.0 },
    }
}
/// A completed dynamic change may precede the driver's current-mode query.
/// Wait only at the exact previous mode; a third mode is never ours to overwrite.
fn display_write_verified(
    expected: DisplayMode,
    desired: DisplayMode,
    actual: DisplayMode,
) -> Result<bool, GamingError> {
    if actual == desired {
        Ok(true)
    } else if actual == expected {
        Ok(false)
    } else {
        Err(GamingError::Changed)
    }
}
fn verify_display_write(
    target: &DisplayTarget,
    expected: DisplayMode,
    desired: DisplayMode,
) -> Result<(), GamingError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        verify_display(target)?;
        let actual = mode_value(&raw_mode(target, ENUM_CURRENT_SETTINGS)?);
        #[cfg(feature = "tuning-test-controls")]
        eprintln!(
            "Display post-apply verification: expected={expected:?} desired={desired:?} actual={actual:?}"
        );
        if display_write_verified(expected, desired, actual)? {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(GamingError::Changed);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
pub fn display_modes(target: &DisplayTarget) -> Result<DisplayModes, GamingError> {
    verify_display(target)?;
    let current = mode_value(&raw_mode(target, ENUM_CURRENT_SETTINGS)?);
    if current.width == 0
        || current.height == 0
        || current.bits_per_pixel != 32
        || !(2..=1000).contains(&current.frequency)
    {
        return Err(GamingError::Unsupported(
            "Current resolution, 32-bit color or nominal refresh cannot be verified",
        ));
    }
    let mut compatible = Vec::new();
    let mut exhausted = false;
    let name = wide(&target.device_name);
    for index in 0..4096 {
        let Ok(mut raw) = raw_mode(target, ENUM_DISPLAY_SETTINGS_MODE(index)) else {
            exhausted = true;
            break;
        };
        let mode = mode_value(&raw);
        if current.compatible(mode) && !compatible.contains(&mode) {
            raw.dmFields = DM_DISPLAYFREQUENCY;
            raw.dmDriverExtra = 0;
            if unsafe {
                ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&raw), None, CDS_TEST, None)
            } == DISP_CHANGE_SUCCESSFUL
            {
                compatible.push(mode);
            }
        }
    }
    if !exhausted {
        return Err(GamingError::Unsupported(
            "Display mode inventory exceeded its bound",
        ));
    }
    if compatible.is_empty() {
        return Err(GamingError::Unsupported(
            "No compatible refresh mode passed the Windows display test",
        ));
    }
    verify_display(target)?;
    if mode_value(&raw_mode(target, ENUM_CURRENT_SETTINGS)?) != current {
        return Err(GamingError::Changed);
    }
    Ok(DisplayModes {
        current,
        compatible,
    })
}
fn validate_display_transition(
    modes: &DisplayModes,
    expected: DisplayMode,
    desired: DisplayMode,
) -> Result<(), GamingError> {
    if modes.current != expected || !expected.compatible(desired) {
        return Err(GamingError::Changed);
    }
    if !modes.compatible.contains(&desired) {
        return Err(GamingError::Unsupported(
            "Refresh mode is no longer available",
        ));
    }
    Ok(())
}
pub fn write_display_mode(
    target: &DisplayTarget,
    expected: DisplayMode,
    desired: DisplayMode,
) -> Result<(), GamingError> {
    let modes = display_modes(target)?;
    validate_display_transition(&modes, expected, desired)?;
    let mut selected = None;
    for index in 0..4096 {
        let Ok(mode) = raw_mode(target, ENUM_DISPLAY_SETTINGS_MODE(index)) else {
            break;
        };
        if mode_value(&mode) == desired {
            selected = Some(mode);
            break;
        }
    }
    let mut mode = selected.ok_or(GamingError::Changed)?;
    // Only refresh is changed. All compatible geometry/color/scan fields were
    // checked above; desktop position, HDR and driver-private data are untouched.
    mode.dmFields = DM_DISPLAYFREQUENCY;
    mode.dmDriverExtra = 0;
    let name = wide(&target.device_name);
    let tested = unsafe {
        ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&mode), None, CDS_TEST, None)
    };
    if tested != DISP_CHANGE_SUCCESSFUL {
        return Err(GamingError::Native {
            operation: "test display refresh",
            code: tested.0 as u32,
        });
    }
    verify_display(target)?;
    if mode_value(&raw_mode(target, ENUM_CURRENT_SETTINGS)?) != expected {
        return Err(GamingError::Changed);
    }
    // Dynamic, user-session mode only: never publish an unconfirmed persistent
    // registry mode. The application worker owns the bounded confirm/undo timer.
    let applied = unsafe {
        ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&mode), None, CDS_TYPE(0), None)
    };
    if applied != DISP_CHANGE_SUCCESSFUL {
        return Err(GamingError::Native {
            operation: "apply display refresh",
            code: applied.0 as u32,
        });
    }
    verify_display_write(target, expected, desired)
}

#[cfg(test)]
mod display_verification_tests {
    use super::*;

    #[test]
    fn gpu_owned_path_remains_valid_after_executable_removal() {
        let path = std::env::temp_dir().join(format!(
            "cpu-affinity-removed-gpu-{}.exe",
            std::process::id()
        ));
        assert!(!path.exists());
        let path = path.to_str().unwrap();
        assert_eq!(owned_gpu_path(path), Ok(path.to_owned()));
        assert!(
            verified_executable(path).is_err(),
            "new activation still requires an existing EXE"
        );
        assert_eq!(
            owned_gpu_path(r"\\server\share\removed.exe"),
            Ok(r"\\server\share\removed.exe".into())
        );
        for invalid in [
            "game.exe",
            r"C:\game.txt",
            "C:\\game.exe\0",
            r"C:\Games\..\game.exe",
            r"\\?\C:\game.exe",
            r"C:game.exe",
        ] {
            assert!(owned_gpu_path(invalid).is_err());
        }
    }

    #[test]
    fn gpu_preference_path_normalizes_verbatim_unc_without_losing_root() {
        assert_eq!(
            preference_executable_path(r"\\?\UNC\server\share\game.exe"),
            r"\\server\share\game.exe"
        );
        assert_eq!(
            preference_executable_path(r"\\?\C:\Games\game.exe"),
            r"C:\Games\game.exe"
        );
        assert_eq!(
            preference_executable_path(r"C:\Games\game.exe"),
            r"C:\Games\game.exe"
        );
    }

    #[test]
    fn display_restore_requires_tested_destination_without_requiring_current_inventory() {
        let current = mode(144);
        let baseline = mode(75);
        let modes = DisplayModes {
            current,
            compatible: vec![baseline],
        };
        assert_eq!(
            validate_display_transition(&modes, current, baseline),
            Ok(())
        );
        assert!(validate_display_transition(&modes, current, mode(60)).is_err());
        assert!(validate_display_transition(&modes, mode(120), baseline).is_err());
    }

    fn mode(frequency: u32) -> DisplayMode {
        DisplayMode {
            width: 3840,
            height: 2160,
            bits_per_pixel: 32,
            frequency,
            flags: 0,
            orientation: 0,
            fixed_output: 0,
        }
    }

    #[test]
    fn display_post_apply_readback_retries_only_exact_previous_mode() {
        let expected = mode(24);
        let desired = mode(144);
        assert_eq!(
            display_write_verified(expected, desired, expected),
            Ok(false)
        );
        assert_eq!(display_write_verified(expected, desired, desired), Ok(true));
        assert_eq!(
            display_write_verified(expected, desired, mode(60)),
            Err(GamingError::Changed)
        );
        assert_eq!(
            display_write_verified(
                expected,
                desired,
                DisplayMode {
                    width: 1920,
                    ..desired
                }
            ),
            Err(GamingError::Changed)
        );
        assert_eq!(
            display_write_verified(
                expected,
                desired,
                DisplayMode {
                    flags: 2,
                    ..desired
                }
            ),
            Err(GamingError::Changed)
        );
    }
}
