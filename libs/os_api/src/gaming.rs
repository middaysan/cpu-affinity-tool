//! Small typed Windows gaming-setting values. Native functions live in Windows.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GamingError {
    Unsupported(&'static str),
    Denied { operation: &'static str, code: u32 },
    Native { operation: &'static str, code: u32 },
    Changed,
    InvalidValue,
}
impl std::fmt::Display for GamingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for GamingError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserIdentity {
    pub sid: String,
    pub logon: u64,
}
/// Bounded canonical decimal SID syntax. This supplies no native account authority.
pub fn valid_account_sid(sid: &str) -> bool {
    fn component(value: &str, maximum: u64) -> bool {
        !value.is_empty()
            && (value == "0" || !value.starts_with('0'))
            && value.bytes().all(|b| b.is_ascii_digit())
            && value.parse::<u64>().is_ok_and(|value| value <= maximum)
    }
    if sid.len() > 184 {
        return false;
    }
    let mut parts = sid.split('-');
    if parts.next() != Some("S")
        || parts.next() != Some("1")
        || !parts
            .next()
            .is_some_and(|value| component(value, (1 << 48) - 1))
    {
        return false;
    }
    let mut count = 0;
    for value in parts {
        count += 1;
        if count > 15 || !component(value, u32::MAX as u64) {
            return false;
        }
    }
    count != 0
}
#[cfg(any(windows, test))]
pub(crate) fn verified_session_user(
    expected: &UserIdentity,
    actual: Result<UserIdentity, GamingError>,
) -> Result<UserIdentity, GamingError> {
    let actual = actual?;
    if expected == &actual {
        Ok(actual)
    } else {
        Err(GamingError::Changed)
    }
}
#[cfg(any(windows, test))]
pub(crate) fn admitted_account_user(
    expected: &UserIdentity,
    actual: Result<UserIdentity, GamingError>,
) -> Result<UserIdentity, GamingError> {
    let actual = actual?;
    if expected.sid == actual.sid && actual.logon != 0 && valid_account_sid(&actual.sid) {
        Ok(actual)
    } else {
        Err(GamingError::Changed)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_token: u64,
    pub image_path: String,
}
/// Selects the classic fields that the launch/monitor path still owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessSettingsFields {
    pub affinity: bool,
    pub priority: bool,
}
impl Default for ProcessSettingsFields {
    fn default() -> Self {
        Self {
            affinity: true,
            priority: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessLaunch {
    pub pid: u32,
    pub creation_token: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GuidValue(pub [u8; 16]);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerScheme {
    pub id: GuidValue,
    pub name: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Dc,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseParameters(pub [i32; 3]);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityFeature {
    Sticky,
    Filter,
    Toggle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityState {
    pub enabled: bool,
    pub hotkey_flags: u32,
}
/// Readonly full-field witness for opt-in native rollback tests, excluded from shipping.
#[cfg(feature = "tuning-test-controls")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserSettingValidation {
    pub mouse_speed: i32,
    pub sticky_flags: u32,
    pub filter_flags: u32,
    pub filter_wait_ms: u32,
    pub filter_delay_ms: u32,
    pub filter_repeat_ms: u32,
    pub filter_bounce_ms: u32,
    pub toggle_flags: u32,
}
pub const ACCESSIBILITY_HOTKEY_MASK: u32 = 0x0c;
pub const EXECUTION_SPEED: u32 = 1;
pub const IGNORE_TIMER_RESOLUTION: u32 = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerThrottling {
    pub version: u32,
    pub control_mask: u32,
    pub state_mask: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedPolicy {
    pub control: u32,
    pub state: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuSet {
    pub id: u32,
    pub group: u16,
    pub logical_processor: u8,
    pub core: u8,
    pub efficiency_class: u8,
    pub parked: bool,
    pub allocated: bool,
    pub allocated_to_target: bool,
}

pub fn merge_policy(
    current: PowerThrottling,
    bit: u32,
    desired: OwnedPolicy,
) -> Result<PowerThrottling, GamingError> {
    if current.version != 1
        || !matches!(bit, EXECUTION_SPEED | IGNORE_TIMER_RESOLUTION)
        || (desired.control | desired.state) & !bit != 0
    {
        return Err(GamingError::InvalidValue);
    }
    Ok(PowerThrottling {
        version: current.version,
        control_mask: (current.control_mask & !bit) | desired.control,
        state_mask: (current.state_mask & !bit) | desired.state,
    })
}
pub fn merge_hotkeys(current_flags: u32, desired: u32) -> Result<u32, GamingError> {
    if desired & !ACCESSIBILITY_HOTKEY_MASK != 0 {
        return Err(GamingError::InvalidValue);
    }
    Ok((current_flags & !ACCESSIBILITY_HOTKEY_MASK) | desired)
}
pub fn validate_cpu_set_selection(ids: &[u32], topology: &[CpuSet]) -> Result<(), GamingError> {
    if ids.len() > 16384
        || ids.iter().enumerate().any(|(index, id)| {
            ids[..index].contains(id)
                || !topology
                    .iter()
                    .any(|cpu| cpu.id == *id && (!cpu.allocated || cpu.allocated_to_target))
        })
    {
        return Err(GamingError::InvalidValue);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ModestPriority {
    Idle,
    BelowNormal,
    #[default]
    Normal,
    AboveNormal,
    High,
}
impl ModestPriority {
    pub fn raw(self) -> u32 {
        match self {
            Self::Idle => 0x40,
            Self::BelowNormal => 0x4000,
            Self::Normal => 0x20,
            Self::AboveNormal => 0x8000,
            Self::High => 0x80,
        }
    }
}
pub const POWER_MODE_BALANCED: GuidValue = GuidValue([0; 16]);
pub const POWER_MODE_EFFICIENCY: GuidValue =
    GuidValue(0x961cc777_2547_4f9d_8174_7d86181b8a7au128.to_be_bytes());
pub const POWER_MODE_PERFORMANCE: GuidValue =
    GuidValue(0xded574b5_45a0_4f42_8737_46345c09c238u128.to_be_bytes());

#[cfg(all(windows, feature = "tuning-test-controls"))]
pub use crate::windows::gaming::read_user_setting_validation;
#[cfg(windows)]
pub use crate::windows::gaming::{
    cpu_topology, interactive_user, power_schemes, read_accessibility, read_cpu_sets, read_mouse,
    read_power_mode, read_power_plan, read_priority, read_process_policy, write_accessibility,
    write_cpu_sets, write_mouse, write_power_mode, write_power_plan, write_priority,
    write_process_policy,
};

#[cfg(not(windows))]
mod unavailable {
    use super::*;
    macro_rules! functions {
        ($(fn $name:ident($($argument:ident:$argument_type:ty),*) -> $value:ty;)+) => {$(
            pub fn $name($($argument:$argument_type),*) -> Result<$value, GamingError> {
                $(let _ = $argument;)*
                Err(GamingError::Unsupported("This module requires Windows"))
            }
        )+};
    }
    functions! {
        fn interactive_user() -> UserIdentity;
        fn power_schemes(user:&UserIdentity) -> Vec<PowerScheme>;
        fn read_power_plan(user:&UserIdentity) -> GuidValue;
        fn write_power_plan(user:&UserIdentity, expected:GuidValue, desired:GuidValue) -> ();
        fn read_power_mode(user:&UserIdentity, source:PowerSource) -> GuidValue;
        fn write_power_mode(user:&UserIdentity, source:PowerSource, expected:GuidValue, desired:GuidValue) -> ();
        fn read_mouse(user:&UserIdentity) -> MouseParameters;
        fn write_mouse(user:&UserIdentity, expected:MouseParameters, desired:MouseParameters) -> ();
        fn read_accessibility(user:&UserIdentity, feature:AccessibilityFeature) -> AccessibilityState;
        fn write_accessibility(user:&UserIdentity, feature:AccessibilityFeature, expected:u32, desired:u32) -> ();
        fn read_process_policy(process:&ProcessIdentity, bit:u32) -> OwnedPolicy;
        fn write_process_policy(process:&ProcessIdentity, bit:u32, expected:OwnedPolicy, desired:OwnedPolicy) -> ();
        fn cpu_topology(process:Option<&ProcessIdentity>) -> Vec<CpuSet>;
        fn read_cpu_sets(process:&ProcessIdentity) -> Vec<u32>;
        fn write_cpu_sets(process:&ProcessIdentity, expected:&[u32], desired:&[u32]) -> ();
        fn read_priority(process:&ProcessIdentity) -> u32;
        fn write_priority(process:&ProcessIdentity, expected:u32, desired:u32) -> ();
    }
    #[cfg(feature = "tuning-test-controls")]
    functions! {
        fn read_user_setting_validation(user:&UserIdentity) -> UserSettingValidation;
    }
}
#[cfg(not(windows))]
pub use unavailable::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_account_admission_rebinds_only_the_same_live_account() {
        let old = UserIdentity {
            sid: "S-1-5-21-1".into(),
            logon: 77,
        };
        let current = UserIdentity {
            logon: 78,
            ..old.clone()
        };
        assert_eq!(
            admitted_account_user(&old, Ok(current.clone())),
            Ok(current.clone())
        );
        let canonical = UserIdentity {
            logon: 0,
            ..old.clone()
        };
        assert_eq!(
            admitted_account_user(&canonical, Ok(current.clone())),
            Ok(current.clone())
        );
        let different = UserIdentity {
            sid: "S-1-5-21-2".into(),
            logon: 78,
        };
        assert_eq!(
            admitted_account_user(&old, Ok(different)),
            Err(GamingError::Changed)
        );
        assert_eq!(
            admitted_account_user(&old, Ok(canonical)),
            Err(GamingError::Changed)
        );
    }

    #[test]
    fn durable_account_admission_preserves_native_denial_and_exact_session_checks() {
        let old = UserIdentity {
            sid: "S-1-5-21-1".into(),
            logon: 77,
        };
        let current = UserIdentity {
            logon: 78,
            ..old.clone()
        };
        assert_eq!(
            verified_session_user(&old, Ok(current)),
            Err(GamingError::Changed)
        );
        assert_eq!(
            verified_session_user(&old, Ok(old.clone())),
            Ok(old.clone())
        );
        for failure in [
            GamingError::Unsupported("Per-user settings are unavailable while impersonating"),
            GamingError::Denied {
                operation: "App and Explorer Windows accounts differ",
                code: 5,
            },
            GamingError::Denied {
                operation: "App and its UAC-linked token do not match the Explorer sign-in",
                code: 5,
            },
            GamingError::Changed,
        ] {
            assert_eq!(
                admitted_account_user(&old, Err(failure.clone())),
                Err(failure.clone())
            );
            assert_eq!(
                verified_session_user(&old, Err(failure.clone())),
                Err(failure)
            );
        }
    }

    #[test]
    fn gaming_qos_merge_preserves_timer_policy_and_unknown_bits() {
        let current = PowerThrottling {
            version: 1,
            control_mask: 0x85,
            state_mask: 0x84,
        };
        let merged = merge_policy(
            current,
            EXECUTION_SPEED,
            OwnedPolicy {
                control: 1,
                state: 1,
            },
        )
        .unwrap();
        assert_eq!(
            merged,
            PowerThrottling {
                version: 1,
                control_mask: 0x85,
                state_mask: 0x85
            }
        );
        let restored = merge_policy(
            merged,
            EXECUTION_SPEED,
            OwnedPolicy {
                control: 1,
                state: 0,
            },
        )
        .unwrap();
        assert_eq!(restored, current);
    }
    #[test]
    fn gaming_timer_merge_preserves_qos_and_rejects_foreign_owned_bits() {
        let current = PowerThrottling {
            version: 1,
            control_mask: 0x81,
            state_mask: 0x81,
        };
        let desired = OwnedPolicy {
            control: IGNORE_TIMER_RESOLUTION,
            state: 0,
        };
        let merged = merge_policy(current, IGNORE_TIMER_RESOLUTION, desired).unwrap();
        assert_eq!(merged.control_mask, 0x85);
        assert_eq!(merged.state_mask, 0x81);
        assert_eq!(
            merge_policy(
                current,
                IGNORE_TIMER_RESOLUTION,
                OwnedPolicy {
                    control: 1,
                    state: 0
                }
            ),
            Err(GamingError::InvalidValue)
        );
    }
    #[test]
    fn gaming_hotkey_merge_preserves_feature_and_unrelated_flags() {
        assert_eq!(merge_hotkeys(0x731f, 0).unwrap(), 0x7313);
        assert_eq!(merge_hotkeys(0x7313, 0x0c).unwrap(), 0x731f);
        assert_eq!(merge_hotkeys(0x7313, 1), Err(GamingError::InvalidValue));
    }
    #[test]
    fn gaming_cpu_sets_accept_empty_original_and_use_real_ids_not_indices() {
        let topology = vec![CpuSet {
            id: 257,
            group: 1,
            logical_processor: 3,
            core: 2,
            efficiency_class: 9,
            parked: false,
            allocated: false,
            allocated_to_target: false,
        }];
        assert!(validate_cpu_set_selection(&[], &topology).is_ok());
        assert!(validate_cpu_set_selection(&[257], &topology).is_ok());
        assert_eq!(
            validate_cpu_set_selection(&[3], &topology),
            Err(GamingError::InvalidValue)
        );
        assert_eq!(
            validate_cpu_set_selection(&[257, 257], &topology),
            Err(GamingError::InvalidValue)
        );
        let mut unavailable = topology;
        unavailable[0].allocated = true;
        assert_eq!(
            validate_cpu_set_selection(&[257], &unavailable),
            Err(GamingError::InvalidValue)
        );
    }
    #[test]
    fn gaming_unknown_policy_versions_cannot_be_merged_or_recaptured() {
        let current = PowerThrottling {
            version: 2,
            control_mask: 1,
            state_mask: 1,
        };
        assert_eq!(
            merge_policy(
                current,
                EXECUTION_SPEED,
                OwnedPolicy {
                    control: 1,
                    state: 0
                }
            ),
            Err(GamingError::InvalidValue)
        );
        assert_eq!(
            merge_policy(
                PowerThrottling {
                    version: 1,
                    ..current
                },
                0x80,
                OwnedPolicy {
                    control: 0x80,
                    state: 0
                }
            ),
            Err(GamingError::InvalidValue)
        );
    }
    #[test]
    fn gaming_modest_priority_config_cannot_request_realtime() {
        assert!(serde_json::from_str::<ModestPriority>("\"Realtime\"").is_err());
        for priority in [
            ModestPriority::Idle,
            ModestPriority::BelowNormal,
            ModestPriority::Normal,
            ModestPriority::AboveNormal,
            ModestPriority::High,
        ] {
            assert_ne!(priority.raw(), 0x100);
        }
    }
}
