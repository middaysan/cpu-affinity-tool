//! Bounded native inventory. These methods make no setting or device writes.
use super::{OS, common::ComGuard};
use crate::gaming_diagnostics::*;
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::*,
            FunctionDiscovery::{PKEY_Device_FriendlyName, PKEY_Device_InstanceId},
        },
        Foundation::{ERROR_NO_MORE_ITEMS, PROPERTYKEY, S_OK},
        Graphics::Gdi::*,
        Media::Audio::*,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, STGM_READ,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            },
            Registry::{HKEY, REG_VALUE_TYPE, RegQueryValueExW},
            Threading::{GetActiveProcessorCount, GetActiveProcessorGroupCount},
        },
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{Interface, PCWSTR, PWSTR, w},
};
use winreg::{
    RegKey,
    enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
};

const MAX_DISPLAYS: u32 = 32;
const MAX_ENDPOINTS: u32 = 128;
const MAX_SESSIONS: usize = 256;
const MAX_DEVICES: u32 = 4096;
const MAX_TEXT_UNITS: usize = 1024;

fn native_error(operation: &'static str, error: windows::core::Error) -> DiagnosticError {
    DiagnosticError::Native {
        operation,
        code: i64::from(error.code().0),
    }
}
fn wide_text(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}
/// COM strings are allocated by the callee. Bound scanning, then always release.
unsafe fn owned_com_text(value: PWSTR) -> Result<String, DiagnosticError> {
    if value.is_null() {
        return Err(DiagnosticError::Native {
            operation: "COM text",
            code: 0,
        });
    }
    let mut units = Vec::new();
    let mut terminated = false;
    for index in 0..MAX_TEXT_UNITS {
        // The COM contract supplies a NUL-terminated string owned by the callee.
        let unit = unsafe { *value.0.add(index) };
        if unit == 0 {
            terminated = true;
            break;
        }
        units.push(unit);
    }
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    if !terminated {
        return Err(DiagnosticError::Limit {
            operation: "COM text",
        });
    }
    Ok(String::from_utf16_lossy(&units))
}

struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn device_text(
    set: HDEVINFO,
    info: &SP_DEVINFO_DATA,
    property: SETUP_DI_REGISTRY_PROPERTY,
) -> Option<String> {
    let mut bytes = [0u8; MAX_TEXT_UNITS * 2];
    let mut value_type = 0;
    let mut required = 0;
    // Optional missing properties stay absent. They never establish a driver,
    // transport or relationship through a guessed friendly-name fallback.
    unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            info,
            property,
            Some(&mut value_type),
            Some(&mut bytes),
            Some(&mut required),
        )
        .ok()?;
    }
    if value_type != 1 || required < 2 || required as usize > bytes.len() || required % 2 != 0 {
        return None;
    }
    let units: Vec<_> = bytes[..required as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if units.last() != Some(&0) {
        return None;
    }
    Some(wide_text(&units))
}

fn stored_value(instance: &str, subkey: &str, value_name: &str) -> StoredInterruptValue {
    let path = format!(
        "SYSTEM\\CurrentControlSet\\Enum\\{instance}\\Device Parameters\\Interrupt Management\\{subkey}"
    );
    let key = match RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY)
    {
        Ok(key) => key,
        Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => {
            return StoredInterruptValue::Absent;
        }
        Err(error) => {
            return StoredInterruptValue::Unavailable {
                code: i64::from(error.raw_os_error().unwrap_or(-1)),
            };
        }
    };
    let name: Vec<u16> = value_name.encode_utf16().chain([0]).collect();
    let mut bytes = [0u8; 64];
    let mut byte_count = bytes.len() as u32;
    let mut value_type = REG_VALUE_TYPE(0);
    let status = unsafe {
        RegQueryValueExW(
            HKEY(key.raw_handle().cast()),
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(bytes.as_mut_ptr()),
            Some(&mut byte_count),
        )
    };
    interrupt_value(status.0, value_type.0, &bytes, byte_count)
}
fn interrupt_value(
    status: u32,
    value_type: u32,
    bytes: &[u8],
    byte_count: u32,
) -> StoredInterruptValue {
    match status {
        0 if byte_count as usize <= bytes.len() && byte_count <= 64 => {
            StoredInterruptValue::Present {
                value_type,
                bytes: bytes[..byte_count as usize].to_vec(),
            }
        }
        0 => StoredInterruptValue::Unavailable { code: 234 },
        2 | 3 => StoredInterruptValue::Absent,
        code => StoredInterruptValue::Unavailable {
            code: i64::from(code),
        },
    }
}
fn has_policy(policy: &DeviceInterruptPolicy) -> bool {
    [
        &policy.affinity_policy,
        &policy.assignment_set,
        &policy.priority,
        &policy.msi_supported,
    ]
    .iter()
    .any(|value| !matches!(value, StoredInterruptValue::Absent))
}

fn endpoint_property(endpoint: &IMMDevice, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let store = endpoint.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(key).ok()?;
        let text = PropVariantToStringAlloc(&value)
            .ok()
            .and_then(|text| owned_com_text(text).ok());
        let _ = PropVariantClear(&mut value);
        text
    }
}

fn sessions(
    endpoint: &IMMDevice,
    remaining: &mut usize,
) -> Result<Vec<AudioSession>, DiagnosticError> {
    unsafe {
        let manager: IAudioSessionManager2 = endpoint
            .Activate(CLSCTX_INPROC_SERVER, None)
            .map_err(|error| native_error("audio session manager", error))?;
        let enumerator = manager
            .GetSessionEnumerator()
            .map_err(|error| native_error("audio sessions", error))?;
        let count = enumerator
            .GetCount()
            .map_err(|error| native_error("audio session count", error))?;
        if count < 0 || count as usize > *remaining {
            return Err(DiagnosticError::Limit {
                operation: "audio sessions",
            });
        }
        *remaining -= count as usize;
        let mut result = Vec::new();
        for index in 0..count {
            let session = enumerator
                .GetSession(index)
                .map_err(|error| native_error("audio session", error))?;
            let session: IAudioSessionControl2 = session
                .cast()
                .map_err(|error| native_error("audio session client", error))?;
            let mut pid = 0;
            // Preserve the positive AUDCLNT_S_NO_SINGLE_PROCESS status that the
            // generated GetProcessId Result wrapper converts into ordinary Ok.
            let status =
                (Interface::vtable(&session).GetProcessId)(Interface::as_raw(&session), &mut pid);
            let client = audio_client(status.0, pid, session.IsSystemSoundsSession() == S_OK);
            let state = match session.GetState() {
                Ok(state) if state == AudioSessionStateActive => AudioState::Active,
                Ok(state) if state == AudioSessionStateInactive => AudioState::Inactive,
                Ok(state) if state == AudioSessionStateExpired => AudioState::Expired,
                _ => AudioState::Unavailable,
            };
            result.push(AudioSession { client, state });
        }
        Ok(result)
    }
}

impl OS {
    pub(crate) fn gaming_display_inventory() -> Result<Vec<DisplayInfo>, DiagnosticError> {
        let mut result = Vec::new();
        for index in 0..MAX_DISPLAYS {
            let mut adapter = DISPLAY_DEVICEW {
                cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut adapter, 0) }.as_bool() {
                return Ok(result);
            }
            if !adapter
                .StateFlags
                .contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
            {
                continue;
            }
            let name = PCWSTR(adapter.DeviceName.as_ptr());
            let mut mode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            if !unsafe {
                EnumDisplaySettingsExW(
                    name,
                    ENUM_CURRENT_SETTINGS,
                    &mut mode,
                    ENUM_DISPLAY_SETTINGS_FLAGS(0),
                )
            }
            .as_bool()
            {
                return Err(DiagnosticError::Native {
                    operation: "current display mode",
                    code: i64::from(windows::core::Error::from_thread().code().0),
                });
            }
            let mut monitor = DISPLAY_DEVICEW {
                cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            let monitor_name = unsafe { EnumDisplayDevicesW(name, 0, &mut monitor, 0) }
                .as_bool()
                .then(|| wide_text(&monitor.DeviceString));
            result.push(DisplayInfo {
                device_name: wide_text(&adapter.DeviceName),
                adapter_name: wide_text(&adapter.DeviceString),
                monitor_name,
                width: mode.dmPelsWidth,
                height: mode.dmPelsHeight,
                nominal_refresh_hz: (mode.dmDisplayFrequency > 1)
                    .then_some(mode.dmDisplayFrequency),
                primary: adapter.StateFlags.contains(DISPLAY_DEVICE_PRIMARY_DEVICE),
            });
        }
        Err(DiagnosticError::Limit {
            operation: "display adapters",
        })
    }

    pub(crate) fn gaming_audio_inventory() -> Result<Vec<AudioEndpoint>, DiagnosticError> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|error| native_error("initialize audio COM", error))?;
        }
        let _com = ComGuard;
        let mut result = Vec::new();
        let mut remaining = MAX_SESSIONS;
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| native_error("audio endpoints", error))?;
            for (flow, direction) in [
                (eRender, AudioDirection::Render),
                (eCapture, AudioDirection::Capture),
            ] {
                let collection = enumerator
                    .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
                    .map_err(|error| native_error("enumerate audio endpoints", error))?;
                let count = collection
                    .GetCount()
                    .map_err(|error| native_error("audio endpoint count", error))?;
                if count > MAX_ENDPOINTS || result.len() + count as usize > MAX_ENDPOINTS as usize {
                    return Err(DiagnosticError::Limit {
                        operation: "audio endpoints",
                    });
                }
                let default_id = enumerator
                    .GetDefaultAudioEndpoint(flow, eMultimedia)
                    .ok()
                    .and_then(|endpoint| endpoint.GetId().ok())
                    .and_then(|id| owned_com_text(id).ok());
                for index in 0..count {
                    let endpoint = collection
                        .Item(index)
                        .map_err(|error| native_error("audio endpoint", error))?;
                    let id = owned_com_text(
                        endpoint
                            .GetId()
                            .map_err(|error| native_error("audio endpoint ID", error))?,
                    )?;
                    let (sessions, session_error) = match sessions(&endpoint, &mut remaining) {
                        Ok(sessions) => (sessions, None),
                        Err(error) => (Vec::new(), Some(error)),
                    };
                    result.push(AudioEndpoint {
                        default_multimedia: default_id.as_ref() == Some(&id),
                        id,
                        name: endpoint_property(&endpoint, &PKEY_Device_FriendlyName),
                        pnp_instance_id: endpoint_property(&endpoint, &PKEY_Device_InstanceId),
                        direction,
                        sessions,
                        session_error,
                    });
                }
            }
        }
        Ok(result)
    }

    pub(crate) fn gaming_pnp_inventory() -> Result<PnpInventory, DiagnosticError> {
        let set = DeviceSet(
            unsafe {
                SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)
            }
            .map_err(|error| native_error("present PnP devices", error))?,
        );
        let mut devices = Vec::new();
        let mut interrupt_policies = Vec::new();
        let mut complete = false;
        for index in 0..MAX_DEVICES {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if let Err(error) = unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut info) } {
                if error.code() == windows::core::HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) {
                    complete = true;
                    break;
                }
                return Err(native_error("enumerate PnP device", error));
            }
            let mut id = [0u16; MAX_TEXT_UNITS];
            unsafe { SetupDiGetDeviceInstanceIdW(set.0, &info, Some(&mut id), None) }
                .map_err(|error| native_error("PnP instance ID", error))?;
            let instance_id = wide_text(&id);
            let mut parent_node = 0;
            let mut parent_id = [0u16; MAX_TEXT_UNITS];
            let parent_id = if unsafe { CM_Get_Parent(&mut parent_node, info.DevInst, 0) }
                == CR_SUCCESS
                && unsafe { CM_Get_Device_IDW(parent_node, &mut parent_id, 0) } == CR_SUCCESS
            {
                Some(wide_text(&parent_id))
            } else {
                None
            };
            let policy = DeviceInterruptPolicy {
                affinity_policy: stored_value(&instance_id, "Affinity Policy", "DevicePolicy"),
                assignment_set: stored_value(
                    &instance_id,
                    "Affinity Policy",
                    "AssignmentSetOverride",
                ),
                priority: stored_value(&instance_id, "Affinity Policy", "DevicePriority"),
                msi_supported: stored_value(
                    &instance_id,
                    "MessageSignaledInterruptProperties",
                    "MSISupported",
                ),
                instance_id: instance_id.clone(),
            };
            if has_policy(&policy) {
                interrupt_policies.push(policy);
            }
            devices.push(PnpDevice {
                instance_id,
                parent_id,
                name: device_text(set.0, &info, SPDRP_FRIENDLYNAME)
                    .or_else(|| device_text(set.0, &info, SPDRP_DEVICEDESC))
                    .unwrap_or_else(|| "Device name unavailable".into()),
                class: device_text(set.0, &info, SPDRP_CLASS).unwrap_or_default(),
                service: device_text(set.0, &info, SPDRP_SERVICE).unwrap_or_default(),
                enumerator: device_text(set.0, &info, SPDRP_ENUMERATOR_NAME).unwrap_or_default(),
                location: device_text(set.0, &info, SPDRP_LOCATION_INFORMATION),
            });
        }
        if !complete {
            return Err(DiagnosticError::Limit {
                operation: "PnP devices",
            });
        }
        let group_count = unsafe { GetActiveProcessorGroupCount() };
        if group_count == 0 || group_count > 64 {
            return Err(DiagnosticError::Limit {
                operation: "processor groups",
            });
        }
        let processor_groups = (0..group_count)
            .map(|group| ProcessorGroup {
                group,
                logical_processors: unsafe { GetActiveProcessorCount(group) },
            })
            .collect();
        Ok(PnpInventory {
            devices,
            interrupt_policies,
            processor_groups,
        })
    }

    pub(crate) fn gaming_open_settings(page: SettingsPage) -> Result<(), DiagnosticError> {
        let uri: Vec<u16> = page.uri().encode_utf16().chain([0]).collect();
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(uri.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        let code = result.0 as isize;
        if code <= 32 {
            Err(DiagnosticError::Native {
                operation: "open native Settings",
                code: code as i64,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Explicit read-only inventory check on the current Windows desktop"]
    fn gaming_native_readonly_inventory_reports_actual_display_audio_and_pnp() {
        let displays = OS::gaming_display_inventory().expect("current display inventory");
        let audio = OS::gaming_audio_inventory().expect("current audio endpoints and sessions");
        let pnp = OS::gaming_pnp_inventory().expect("current present PnP inventory");
        assert!(!displays.is_empty());
        assert!(
            displays
                .iter()
                .all(|display| display.width > 0 && display.height > 0)
        );
        assert!(!pnp.devices.is_empty());
        assert!(!pnp.processor_groups.is_empty());
        assert!(audio.iter().all(|endpoint| !endpoint.id.is_empty()));
        println!(
            "actual_readonly_inventory: displays={}, audio_endpoints={}, present_devices={}, processor_groups={}",
            displays.len(),
            audio.len(),
            pnp.devices.len(),
            pnp.processor_groups.len()
        );
    }

    #[test]
    fn gaming_interrupt_value_keeps_exact_type_bytes_and_bounds_native_result() {
        let bytes = [0xab; 64];
        match interrupt_value(0, 3, &bytes, 4) {
            StoredInterruptValue::Present { value_type, bytes } => {
                assert_eq!(value_type, 3);
                assert_eq!(bytes, [0xab; 4]);
            }
            _ => panic!("expected exact value"),
        }
        assert!(matches!(
            interrupt_value(0, 3, &bytes, 65),
            StoredInterruptValue::Unavailable { code: 234 }
        ));
        assert!(matches!(
            interrupt_value(234, 3, &bytes, 5000),
            StoredInterruptValue::Unavailable { code: 234 }
        ));
        assert!(matches!(
            interrupt_value(2, 3, &bytes, 0),
            StoredInterruptValue::Absent
        ));
        assert!(matches!(
            interrupt_value(5, 3, &bytes, 0),
            StoredInterruptValue::Unavailable { code: 5 }
        ));
    }
}
