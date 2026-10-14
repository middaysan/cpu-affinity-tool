//! Read-only gaming observations and fixed Windows Settings/Security destinations.
//! No audio content, device restarts, driver policies or registry writes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    GameMode,
    BackgroundCapture,
    WindowedOptimizations,
    GpuPreference,
    Display,
    Sound,
    Devices,
    CoreIsolation,
}
impl SettingsPage {
    /// Fixed Microsoft Windows Settings/Windows Security shortcuts; no caller-supplied URI.
    pub const fn uri(self) -> &'static str {
        match self {
            Self::GameMode => "ms-settings:gaming-gamemode",
            Self::BackgroundCapture => "ms-settings:gaming-gamedvr",
            Self::WindowedOptimizations => "ms-settings:display-advancedgraphics-default",
            Self::GpuPreference => "ms-settings:display-advancedgraphics",
            Self::Display => "ms-settings:display",
            Self::Sound => "ms-settings:sound",
            Self::Devices => "ms-settings:connecteddevices",
            Self::CoreIsolation => "windowsdefender://coreisolation/",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticError {
    Unsupported,
    Native { operation: &'static str, code: i64 },
    Limit { operation: &'static str },
}
impl std::fmt::Display for DiagnosticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("This observation requires Windows."),
            Self::Native { operation, code } => {
                write!(f, "{operation} failed (native code {code}).")
            }
            Self::Limit { operation } => {
                write!(f, "{operation} exceeds the bounded inventory limit.")
            }
        }
    }
}
impl std::error::Error for DiagnosticError {}

#[derive(Clone, Debug)]
pub struct DisplayInfo {
    pub device_name: String,
    pub adapter_name: String,
    pub monitor_name: Option<String>,
    pub width: u32,
    pub height: u32,
    /// EnumDisplaySettingsEx reports integer nominal Hz. Zero/one means unknown.
    pub nominal_refresh_hz: Option<u32>,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioClient {
    SingleProcess(u32),
    MultipleProcesses { initial_pid: u32 },
    SystemSounds,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDirection {
    Render,
    Capture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioState {
    Active,
    Inactive,
    Expired,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct AudioSession {
    pub client: AudioClient,
    pub state: AudioState,
}

#[derive(Clone, Debug)]
pub struct AudioEndpoint {
    pub id: String,
    pub name: Option<String>,
    /// Endpoint devnode identity, not a guessed physical driver or audiodg identity.
    pub pnp_instance_id: Option<String>,
    pub direction: AudioDirection,
    pub default_multimedia: bool,
    pub sessions: Vec<AudioSession>,
    pub session_error: Option<DiagnosticError>,
}

#[derive(Clone, Debug)]
pub struct PnpDevice {
    pub instance_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub class: String,
    pub service: String,
    pub enumerator: String,
    pub location: Option<String>,
}

#[derive(Clone, Debug)]
pub enum StoredInterruptValue {
    Absent,
    Present { value_type: u32, bytes: Vec<u8> },
    Unavailable { code: i64 },
}
#[derive(Clone, Debug)]
pub struct DeviceInterruptPolicy {
    pub instance_id: String,
    pub affinity_policy: StoredInterruptValue,
    pub assignment_set: StoredInterruptValue,
    pub priority: StoredInterruptValue,
    pub msi_supported: StoredInterruptValue,
}
#[derive(Clone, Debug)]
pub struct ProcessorGroup {
    pub group: u16,
    pub logical_processors: u32,
}
#[derive(Clone, Debug)]
pub struct PnpInventory {
    pub devices: Vec<PnpDevice>,
    pub processor_groups: Vec<ProcessorGroup>,
    pub interrupt_policies: Vec<DeviceInterruptPolicy>,
}

/// Raw HRESULT is essential: GetProcessId's positive multiprocess status must not
/// be erased by the generated Result<u32> convenience wrapper.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn audio_client(result: i32, pid: u32, system_sounds: bool) -> AudioClient {
    if result < 0 {
        return AudioClient::Unavailable;
    }
    if system_sounds {
        return AudioClient::SystemSounds;
    }
    if result == 0x0889_000d {
        return AudioClient::MultipleProcesses { initial_pid: pid };
    }
    if result == 0 && pid != 0 {
        AudioClient::SingleProcess(pid)
    } else {
        AudioClient::Unavailable
    }
}

pub fn usb_controller<'a>(device: &PnpDevice, devices: &'a [PnpDevice]) -> Option<&'a PnpDevice> {
    let mut parent = device.parent_id.as_deref();
    for _ in 0..32 {
        let found = devices
            .iter()
            .find(|candidate| Some(candidate.instance_id.as_str()) == parent)?;
        // Services identify actual Windows USB host-controller drivers. Friendly
        // names can be localized and never establish a relationship.
        if ["USBXHCI", "USBEHCI", "USBOHCI", "USBUHCI"]
            .iter()
            .any(|service| found.service.eq_ignore_ascii_case(service))
        {
            return Some(found);
        }
        parent = found.parent_id.as_deref();
    }
    None
}

pub fn display_inventory() -> Result<Vec<DisplayInfo>, DiagnosticError> {
    #[cfg(target_os = "windows")]
    {
        crate::OS::gaming_display_inventory()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(DiagnosticError::Unsupported)
    }
}
pub fn audio_inventory() -> Result<Vec<AudioEndpoint>, DiagnosticError> {
    #[cfg(target_os = "windows")]
    {
        crate::OS::gaming_audio_inventory()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(DiagnosticError::Unsupported)
    }
}
pub fn pnp_inventory() -> Result<PnpInventory, DiagnosticError> {
    #[cfg(target_os = "windows")]
    {
        crate::OS::gaming_pnp_inventory()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(DiagnosticError::Unsupported)
    }
}
pub fn open_settings(page: SettingsPage) -> Result<(), DiagnosticError> {
    #[cfg(target_os = "windows")]
    {
        crate::OS::gaming_open_settings(page)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = page;
        Err(DiagnosticError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaming_settings_uris_are_fixed_native_pages() {
        assert_eq!(
            SettingsPage::CoreIsolation.uri(),
            "windowsdefender://coreisolation/"
        );
        assert_eq!(SettingsPage::GameMode.uri(), "ms-settings:gaming-gamemode");
        assert_eq!(
            SettingsPage::BackgroundCapture.uri(),
            "ms-settings:gaming-gamedvr"
        );
        assert_eq!(
            SettingsPage::WindowedOptimizations.uri(),
            "ms-settings:display-advancedgraphics-default"
        );
        assert_eq!(
            SettingsPage::GpuPreference.uri(),
            "ms-settings:display-advancedgraphics"
        );
    }

    #[test]
    fn gaming_usb_relationships_follow_real_parents_and_do_not_guess_by_name() {
        let devices = vec![
            PnpDevice {
                instance_id: "PCI\\HC".into(),
                parent_id: None,
                name: "Controller".into(),
                class: "USB".into(),
                service: "USBXHCI".into(),
                enumerator: "PCI".into(),
                location: None,
            },
            PnpDevice {
                instance_id: "USB\\HUB".into(),
                parent_id: Some("PCI\\HC".into()),
                name: "Hub".into(),
                class: "USB".into(),
                service: "USBHUB3".into(),
                enumerator: "USB".into(),
                location: None,
            },
            PnpDevice {
                instance_id: "HID\\MOUSE".into(),
                parent_id: Some("USB\\HUB".into()),
                name: "Mouse".into(),
                class: "HIDClass".into(),
                service: "mouhid".into(),
                enumerator: "HID".into(),
                location: None,
            },
            PnpDevice {
                instance_id: "HID\\OTHER".into(),
                parent_id: None,
                name: "USB Mouse".into(),
                class: "HIDClass".into(),
                service: "mouhid".into(),
                enumerator: "HID".into(),
                location: None,
            },
        ];
        assert_eq!(
            usb_controller(&devices[2], &devices).unwrap().instance_id,
            "PCI\\HC"
        );
        assert!(usb_controller(&devices[3], &devices).is_none());
    }

    #[test]
    fn gaming_usb_parent_cycle_is_bounded() {
        let devices = vec![PnpDevice {
            instance_id: "USB\\CYCLE".into(),
            parent_id: Some("USB\\CYCLE".into()),
            name: "Device".into(),
            class: "USB".into(),
            service: "".into(),
            enumerator: "USB".into(),
            location: None,
        }];
        assert!(usb_controller(&devices[0], &devices).is_none());
    }

    #[test]
    fn gaming_audio_pid_preserves_multi_process_success_and_failure() {
        assert_eq!(audio_client(0, 42, false), AudioClient::SingleProcess(42));
        assert_eq!(
            audio_client(0x0889000d, 42, false),
            AudioClient::MultipleProcesses { initial_pid: 42 }
        );
        assert_eq!(audio_client(0, 0, true), AudioClient::SystemSounds);
        assert_eq!(
            audio_client(0x80004005_u32 as i32, 42, false),
            AudioClient::Unavailable
        );
        assert_eq!(audio_client(0, 0, false), AudioClient::Unavailable);
    }
}
