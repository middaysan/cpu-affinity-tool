//! Bounded AC processor power requests with exact, restart-stable scheme identities.
#[cfg(not(windows))]
use crate::gaming::UserIdentity;
use crate::gaming::{GamingError, GuidValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuPowerSetting {
    Frequency,
    Frequency1,
    Maximum,
    Maximum1,
    Minimum,
    Minimum1,
    EnergyPreference,
    EnergyPreference1,
    Boost,
    ParkingMinimum,
    ParkingMinimum1,
    ParkingMaximum,
    ParkingMaximum1,
}

/// Independent class limits; absent optional controls retain the old preset behavior.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuPowerOptions {
    pub performance_mhz: u32,
    pub efficient_mhz: Option<u32>,
    pub keep_high: bool,
    pub allow_parking: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuPowerKey {
    pub scheme: GuidValue,
    pub setting: CpuPowerSetting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuPowerRequest {
    pub key: CpuPowerKey,
    pub value: u32,
}

impl CpuPowerSetting {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Frequency => "frequency",
            Self::Frequency1 => "frequency1",
            Self::Maximum => "maximum",
            Self::Maximum1 => "maximum1",
            Self::Minimum => "minimum",
            Self::Minimum1 => "minimum1",
            Self::EnergyPreference => "epp",
            Self::EnergyPreference1 => "epp1",
            Self::Boost => "boost",
            Self::ParkingMinimum => "parking_minimum",
            Self::ParkingMinimum1 => "parking_minimum1",
            Self::ParkingMaximum => "parking_maximum",
            Self::ParkingMaximum1 => "parking_maximum1",
        }
    }
}

impl CpuPowerKey {
    /// An exact scheme/setting identity, never an arbitrary registry path.
    pub fn wire_id(self) -> String {
        format!(
            "cpu_ac_{:032x}_{}",
            u128::from_be_bytes(self.scheme.0),
            self.setting.wire_name()
        )
    }

    pub fn from_wire_id(value: &str) -> Result<Self, GamingError> {
        let (scheme, setting) = value
            .strip_prefix("cpu_ac_")
            .and_then(|value| value.split_once('_'))
            .ok_or(GamingError::InvalidValue)?;
        if scheme.len() != 32
            || !scheme
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(GamingError::InvalidValue);
        }
        let scheme = u128::from_str_radix(scheme, 16).map_err(|_| GamingError::InvalidValue)?;
        if scheme == 0 {
            return Err(GamingError::InvalidValue);
        }
        let setting = match setting {
            "frequency" => CpuPowerSetting::Frequency,
            "frequency1" => CpuPowerSetting::Frequency1,
            "maximum" => CpuPowerSetting::Maximum,
            "maximum1" => CpuPowerSetting::Maximum1,
            "minimum" => CpuPowerSetting::Minimum,
            "minimum1" => CpuPowerSetting::Minimum1,
            "epp" => CpuPowerSetting::EnergyPreference,
            "epp1" => CpuPowerSetting::EnergyPreference1,
            "boost" => CpuPowerSetting::Boost,
            "parking_minimum" => CpuPowerSetting::ParkingMinimum,
            "parking_minimum1" => CpuPowerSetting::ParkingMinimum1,
            "parking_maximum" => CpuPowerSetting::ParkingMaximum,
            "parking_maximum1" => CpuPowerSetting::ParkingMaximum1,
            _ => return Err(GamingError::InvalidValue),
        };
        Ok(Self {
            scheme: GuidValue(scheme.to_be_bytes()),
            setting,
        })
    }
}

#[cfg(any(windows, test))]
pub(crate) fn validate_requested_frequency(value: u32) -> Result<(), GamingError> {
    if value == 0 || ((100..=64000).contains(&value) && value.is_multiple_of(100)) {
        Ok(())
    } else {
        Err(GamingError::InvalidValue)
    }
}

#[cfg(any(windows, test))]
pub(crate) fn validate_scalar(
    setting: CpuPowerSetting,
    value: Option<u32>,
) -> Result<(), GamingError> {
    let valid = match (setting, value) {
        (_, None) | (CpuPowerSetting::Frequency | CpuPowerSetting::Frequency1, Some(_)) => true,
        (CpuPowerSetting::Boost, Some(value)) => value <= 6,
        (_, Some(value)) => value <= 100,
    };
    if valid {
        Ok(())
    } else {
        Err(GamingError::InvalidValue)
    }
}

#[cfg(windows)]
pub use crate::windows::gaming::cpu_power::{
    confirm_value, hybrid_cores, prepare_requests, read_value, write_value,
};

#[cfg(not(windows))]
pub fn hybrid_cores(user: &UserIdentity) -> Result<bool, GamingError> {
    let _ = user;
    Err(GamingError::Unsupported(
        "CPU power settings require Windows",
    ))
}

#[cfg(not(windows))]
pub fn confirm_value(
    user: &UserIdentity,
    key: CpuPowerKey,
    expected: Option<u32>,
) -> Result<(), GamingError> {
    let _ = (user, key, expected);
    Err(GamingError::Unsupported(
        "CPU power settings require Windows",
    ))
}

#[cfg(not(windows))]
pub fn prepare_requests(
    user: &UserIdentity,
    options: &CpuPowerOptions,
) -> Result<Vec<CpuPowerRequest>, GamingError> {
    let _ = (user, options);
    Err(GamingError::Unsupported(
        "CPU power settings require Windows",
    ))
}

/// Returns only the exact stored AC override. `None` means no override exists.
#[cfg(not(windows))]
pub fn read_value(user: &UserIdentity, key: CpuPowerKey) -> Result<Option<u32>, GamingError> {
    let _ = (user, key);
    Err(GamingError::Unsupported(
        "CPU power settings require Windows",
    ))
}

#[cfg(not(windows))]
pub fn write_value(
    user: &UserIdentity,
    key: CpuPowerKey,
    expected: Option<u32>,
    desired: Option<u32>,
) -> Result<(), GamingError> {
    let _ = (user, key, expected, desired);
    Err(GamingError::Unsupported(
        "CPU power settings require Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_binding_codec_rejects_unknown_fields_instead_of_dropping_them() {
        let request = CpuPowerRequest {
            key: CpuPowerKey {
                scheme: SCHEME,
                setting: CpuPowerSetting::Frequency,
            },
            value: 4500,
        };
        let mut encoded = serde_json::to_value(&request).unwrap();
        encoded["future_option"] = true.into();
        assert!(serde_json::from_value::<CpuPowerRequest>(encoded).is_err());
        let mut encoded = serde_json::to_value(&request.key).unwrap();
        encoded["future_identity"] = true.into();
        assert!(serde_json::from_value::<CpuPowerKey>(encoded).is_err());
    }

    const SCHEME: GuidValue = GuidValue(0x381b4222_f694_41f0_9685_ff5bb260df2eu128.to_be_bytes());

    #[test]
    fn exact_cpu_keys_round_trip_every_whitelisted_setting() {
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
            let key = CpuPowerKey {
                scheme: SCHEME,
                setting,
            };
            assert_eq!(CpuPowerKey::from_wire_id(&key.wire_id()), Ok(key));
            assert_eq!(
                serde_json::from_str::<CpuPowerKey>(&serde_json::to_string(&key).unwrap()).unwrap(),
                key
            );
        }
        assert_eq!(
            CpuPowerKey {
                scheme: SCHEME,
                setting: CpuPowerSetting::Frequency
            }
            .wire_id(),
            "cpu_ac_381b4222f69441f09685ff5bb260df2e_frequency"
        );
    }

    #[test]
    fn cpu_key_codec_rejects_aliases_generic_paths_and_foreign_fields() {
        for invalid in [
            "cpu_ac_381B4222f69441f09685ff5bb260df2e_frequency",
            "cpu_ac_381b4222-f694-41f0-9685-ff5bb260df2e_frequency",
            "cpu_ac_381b4222f69441f09685ff5bb260df2e_frequency_dc",
            "cpu_ac_381b4222f69441f09685ff5bb260df2e_parking",
            "cpu_ac_381b4222f69441f09685ff5bb260df2e_frequency/ACSettingIndex",
            "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Power",
            "cpu_ac_00000000000000000000000000000000_frequency",
            "cpu_ac_381b4222f69441f09685ff5bb260df2_frequency",
        ] {
            assert_eq!(
                CpuPowerKey::from_wire_id(invalid),
                Err(GamingError::InvalidValue),
                "{invalid}"
            );
        }
    }

    #[test]
    fn requested_frequency_uses_100_mhz_steps_and_zero_has_no_cap() {
        for frequency in [0, 100, 5400, 64000] {
            assert_eq!(validate_requested_frequency(frequency), Ok(()));
        }
        for frequency in [1, 99, 101, 5401, 64100, u32::MAX] {
            assert_eq!(
                validate_requested_frequency(frequency),
                Err(GamingError::InvalidValue)
            );
        }
    }

    #[test]
    fn exact_original_frequency_does_not_need_to_match_the_ui_step() {
        assert_eq!(
            validate_scalar(CpuPowerSetting::Frequency, Some(5401)),
            Ok(())
        );
        assert_eq!(
            validate_scalar(CpuPowerSetting::Frequency1, Some(u32::MAX)),
            Ok(())
        );
        for setting in [
            CpuPowerSetting::Maximum,
            CpuPowerSetting::Minimum,
            CpuPowerSetting::EnergyPreference,
        ] {
            assert_eq!(validate_scalar(setting, None), Ok(()));
            assert_eq!(validate_scalar(setting, Some(100)), Ok(()));
            assert_eq!(
                validate_scalar(setting, Some(101)),
                Err(GamingError::InvalidValue)
            );
        }
        assert_eq!(validate_scalar(CpuPowerSetting::Boost, Some(6)), Ok(()));
        assert_eq!(
            validate_scalar(CpuPowerSetting::Boost, Some(7)),
            Err(GamingError::InvalidValue)
        );
    }
}
