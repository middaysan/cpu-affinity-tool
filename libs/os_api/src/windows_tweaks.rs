//! Bounded, reversible Windows preference values and display modes.

use crate::gaming::{GamingError, UserIdentity};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredDword {
    Absent,
    Value(u32),
}
impl StoredDword {
    pub fn value(self) -> Option<u32> {
        match self {
            Self::Absent => None,
            Self::Value(value) => Some(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuTarget {
    pub user_sid: String,
    // A nonzero value is legacy session evidence; new account targets omit it.
    #[serde(default, skip_serializing_if = "is_zero_logon")]
    pub user_logon: u64,
    pub executable: String,
}
fn is_zero_logon(logon: &u64) -> bool {
    *logon == 0
}
impl GpuTarget {
    pub fn user(&self) -> UserIdentity {
        UserIdentity {
            sid: self.user_sid.clone(),
            logon: self.user_logon,
        }
    }
}

/// Identity is qualified by the same interactive token and both GDI device IDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayTarget {
    pub user_sid: String,
    pub user_logon: u64,
    pub device_name: String,
    pub adapter_id: String,
    pub monitor_id: String,
}
impl DisplayTarget {
    pub fn user(&self) -> UserIdentity {
        UserIdentity {
            sid: self.user_sid.clone(),
            logon: self.user_logon,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub bits_per_pixel: u32,
    pub frequency: u32,
    pub flags: u32,
    pub orientation: u32,
    pub fixed_output: u32,
}
impl DisplayMode {
    pub fn compatible(self, other: Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.bits_per_pixel == other.bits_per_pixel
            && self.flags == other.flags
            && self.orientation == other.orientation
            && self.fixed_output == other.fixed_output
            && (2..=1000).contains(&other.frequency)
    }
}
#[derive(Clone, Debug)]
pub struct DisplayModes {
    pub current: DisplayMode,
    pub compatible: Vec<DisplayMode>,
}
pub fn highest_compatible_mode(current: DisplayMode, modes: &[DisplayMode]) -> Option<DisplayMode> {
    modes
        .iter()
        .copied()
        .filter(|mode| current.compatible(*mode))
        .max_by_key(|mode| mode.frequency)
}

/// Reject ambiguous, unbounded and malformed compound values. Foreign segments
/// stay byte-for-byte intact; no parser here supplies native activation authority.
fn segments(value: &str) -> Result<Vec<(&str, &str, &str)>, GamingError> {
    if value.is_empty() || value.len() > 16384 || value.chars().any(char::is_control) {
        return Err(GamingError::InvalidValue);
    }
    let mut result: Vec<(&str, &str, &str)> = Vec::new();
    for raw in value.split_inclusive(';') {
        let segment = raw.strip_suffix(';').unwrap_or(raw);
        let (key, field) = segment.split_once('=').ok_or(GamingError::InvalidValue)?;
        if key.is_empty()
            || key.len() > 128
            || field.is_empty()
            || field.len() > 8192
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || result
                .iter()
                .any(|(old, _, _)| old.eq_ignore_ascii_case(key))
        {
            return Err(GamingError::InvalidValue);
        }
        result.push((key, field, raw));
        if result.len() > 128 {
            return Err(GamingError::InvalidValue);
        }
    }
    Ok(result)
}
pub fn preference_field(value: Option<&str>, name: &str) -> Result<Option<String>, GamingError> {
    let Some(value) = value else {
        return Ok(None);
    };
    Ok(segments(value)?
        .into_iter()
        .find(|(key, _, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, field, _)| field.to_owned()))
}
pub fn merge_preference(
    value: Option<&str>,
    name: &str,
    desired: Option<&str>,
) -> Result<Option<String>, GamingError> {
    if !matches!(name, "SwapEffectUpgradeEnable" | "GpuPreference")
        || desired.is_some_and(|v| {
            !matches!(
                (name, v),
                ("SwapEffectUpgradeEnable", "0" | "1") | ("GpuPreference", "0" | "1" | "2")
            )
        })
    {
        return Err(GamingError::InvalidValue);
    }
    let parts = value.map(segments).transpose()?.unwrap_or_default();
    let mut result = String::new();
    let mut replaced = false;
    for (key, _, raw) in parts {
        if key.eq_ignore_ascii_case(name) {
            replaced = true;
            if let Some(desired) = desired {
                result.push_str(key);
                result.push('=');
                result.push_str(desired);
                if raw.ends_with(';') {
                    result.push(';');
                }
            }
        } else {
            result.push_str(raw);
        }
    }
    if !replaced && let Some(desired) = desired {
        let owned = format!("{name}={desired};");
        if !result.is_empty() && !result.ends_with(';') {
            // Prepending avoids adding a separator to a foreign final segment;
            // removing our field later therefore preserves its exact bytes.
            result.insert_str(0, &owned);
        } else {
            result.push_str(&owned);
        }
    }
    if result.len() > 16384 {
        return Err(GamingError::InvalidValue);
    }
    Ok((!result.is_empty()).then_some(result))
}
pub fn windowed_restore_allowed(
    value: Option<&str>,
    desired: Option<&str>,
) -> Result<(), GamingError> {
    if desired != Some("1")
        && preference_field(value, "AutoHDREnable")?.is_some_and(|value| value != "0")
    {
        return Err(GamingError::Unsupported(
            "Auto HDR depends on windowed optimizations; leave it enabled or change Auto HDR in Windows Settings first",
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub use crate::windows::windows_tweaks::{
    display_modes, primary_display_target, qualify_gpu_target, read_background_capture,
    read_game_mode, read_gpu_preference, read_windowed_optimizations, verified_executable,
    write_background_capture, write_display_mode, write_game_mode, write_gpu_preference,
    write_windowed_optimizations,
};

#[cfg(not(target_os = "windows"))]
mod unavailable {
    use super::*;
    fn unavailable<T>() -> Result<T, GamingError> {
        Err(GamingError::Unsupported("Windows Tweaks requires Windows"))
    }
    pub fn read_game_mode(_: &UserIdentity) -> Result<StoredDword, GamingError> {
        unavailable()
    }
    pub fn read_background_capture(_: &UserIdentity) -> Result<StoredDword, GamingError> {
        unavailable()
    }
    pub fn write_game_mode(
        _: &UserIdentity,
        _: StoredDword,
        _: StoredDword,
    ) -> Result<(), GamingError> {
        unavailable()
    }
    pub fn write_background_capture(
        _: &UserIdentity,
        _: StoredDword,
        _: StoredDword,
    ) -> Result<(), GamingError> {
        unavailable()
    }
    pub fn read_windowed_optimizations(_: &UserIdentity) -> Result<Option<String>, GamingError> {
        unavailable()
    }
    pub fn write_windowed_optimizations(
        _: &UserIdentity,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), GamingError> {
        unavailable()
    }
    pub fn verified_executable(_: &str) -> Result<String, GamingError> {
        unavailable()
    }
    pub fn qualify_gpu_target(_: &str) -> Result<GpuTarget, GamingError> {
        unavailable()
    }
    pub fn read_gpu_preference(_: &UserIdentity, _: &str) -> Result<Option<String>, GamingError> {
        unavailable()
    }
    pub fn write_gpu_preference(
        _: &UserIdentity,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), GamingError> {
        unavailable()
    }
    pub fn primary_display_target() -> Result<DisplayTarget, GamingError> {
        unavailable()
    }
    pub fn display_modes(_: &DisplayTarget) -> Result<DisplayModes, GamingError> {
        unavailable()
    }
    pub fn write_display_mode(
        _: &DisplayTarget,
        _: DisplayMode,
        _: DisplayMode,
    ) -> Result<(), GamingError> {
        unavailable()
    }
}
#[cfg(not(target_os = "windows"))]
pub use unavailable::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_account_gpu_codec_accepts_legacy_and_stable_account_targets() {
        let legacy = r#"{"user_sid":"S-1-5-21-1","user_logon":77,"executable":"C:\\game.exe"}"#;
        let stable = r#"{"user_sid":"S-1-5-21-1","executable":"C:\\game.exe"}"#;
        let old: GpuTarget = serde_json::from_str(legacy).unwrap();
        let current: GpuTarget = serde_json::from_str(stable).unwrap();
        assert_eq!(old.user_logon, 77);
        assert_eq!(current.user_logon, 0);
        assert_eq!(old.user_sid, current.user_sid);
        assert_eq!(old.executable, current.executable);
        assert_eq!(serde_json::to_string(&current).unwrap(), stable);
        assert!(
            serde_json::from_str::<DisplayTarget>(
                r#"{"user_sid":"S-1-5-21-1","device_name":"d","adapter_id":"a","monitor_id":"m"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn tweak_dword_absence_is_distinct_from_explicit_off() {
        assert_ne!(StoredDword::Absent, StoredDword::Value(0));
        assert_eq!(StoredDword::Absent.value(), None);
        assert_eq!(StoredDword::Value(1).value(), Some(1));
    }

    #[test]
    fn tweak_composite_merge_preserves_foreign_fields_and_restores_absence() {
        let before = "AutoHDREnable=0;UnknownVendorField=001;";
        let applied = merge_preference(Some(before), "SwapEffectUpgradeEnable", Some("1")).unwrap();
        assert_eq!(
            applied.as_deref(),
            Some("AutoHDREnable=0;UnknownVendorField=001;SwapEffectUpgradeEnable=1;")
        );
        assert_eq!(
            merge_preference(applied.as_deref(), "SwapEffectUpgradeEnable", None)
                .unwrap()
                .as_deref(),
            Some(before)
        );
        let created = merge_preference(None, "GpuPreference", Some("2")).unwrap();
        assert_eq!(
            merge_preference(created.as_deref(), "GpuPreference", None).unwrap(),
            None
        );
        let unterminated = "Foreign=001";
        let applied = merge_preference(Some(unterminated), "GpuPreference", Some("2")).unwrap();
        assert_eq!(
            merge_preference(applied.as_deref(), "GpuPreference", None)
                .unwrap()
                .as_deref(),
            Some(unterminated)
        );
    }

    #[test]
    fn tweak_composite_read_and_merge_reject_ambiguous_or_malformed_values() {
        for invalid in [
            "",
            "GpuPreference=2;GpuPreference=1;",
            "broken",
            "GpuPreference=2;;",
            "GpuPreference=2\0;",
        ] {
            assert!(preference_field(Some(invalid), "GpuPreference").is_err());
            assert!(merge_preference(Some(invalid), "GpuPreference", Some("2")).is_err());
        }
        let almost_full = format!("A={};B={};", "x".repeat(8190), "y".repeat(8180));
        assert!(preference_field(Some(&almost_full), "GpuPreference").is_ok());
        assert!(merge_preference(Some(&almost_full), "GpuPreference", Some("2")).is_err());
    }

    #[test]
    fn tweak_windowed_restore_refuses_autohdr_dependency() {
        assert!(
            windowed_restore_allowed(
                Some("AutoHDREnable=1;SwapEffectUpgradeEnable=1;"),
                Some("0")
            )
            .is_err()
        );
        assert!(
            windowed_restore_allowed(Some("AutoHDREnable=1;SwapEffectUpgradeEnable=1;"), None)
                .is_err()
        );
        assert!(
            windowed_restore_allowed(Some("AutoHDREnable=0;SwapEffectUpgradeEnable=1;"), None)
                .is_ok()
        );
        assert!(
            windowed_restore_allowed(
                Some("AutoHDREnable=1;SwapEffectUpgradeEnable=1;"),
                Some("1")
            )
            .is_ok()
        );
    }

    #[test]
    fn tweak_display_candidate_keeps_resolution_color_and_scan_contract() {
        let current = DisplayMode {
            width: 1920,
            height: 1080,
            bits_per_pixel: 32,
            frequency: 60,
            flags: 0,
            orientation: 0,
            fixed_output: 0,
        };
        let fast = DisplayMode {
            frequency: 144,
            ..current
        };
        let modes = [
            DisplayMode {
                width: 2560,
                frequency: 240,
                ..current
            },
            DisplayMode {
                bits_per_pixel: 16,
                frequency: 240,
                ..current
            },
            DisplayMode {
                flags: 2,
                frequency: 240,
                ..current
            },
            fast,
            current,
        ];
        assert_eq!(highest_compatible_mode(current, &modes), Some(fast));
        assert_eq!(
            highest_compatible_mode(
                current,
                &[DisplayMode {
                    frequency: 0,
                    ..current
                }]
            ),
            None
        );
    }
}
