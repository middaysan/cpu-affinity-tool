//! Shared bounded persisted configuration and field token formatting.
use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{codec::ModuleEnvelope, contract::*},
    presentation::ModuleAction,
};
use os_api::{
    gaming_diagnostics::{self, SettingsPage},
    windows_tweaks::{DisplayMode, StoredDword},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) recommended: bool,
    pub(super) refresh: Option<DisplayMode>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            recommended: true,
            refresh: None,
        }
    }
}

pub(super) fn config(
    id: &str,
    envelope: &ModuleEnvelope,
    supports_refresh: bool,
) -> Result<Config, ModuleError> {
    let value: Config = decode(id, envelope)?;
    if !value.recommended
        || (!supports_refresh && value.refresh.is_some())
        || value.refresh.is_some_and(|mode| {
            mode.width == 0
                || mode.height == 0
                || mode.bits_per_pixel != 32
                || !(2..=1000).contains(&mode.frequency)
                || mode.orientation > 3
                || mode.fixed_output > 2
        })
    {
        return Err(error(
            ErrorKind::InvalidConfig,
            "Windows tweak configuration",
        ));
    }
    Ok(value)
}

pub(super) fn token<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("bounded typed field")
}
pub(super) fn value<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ModuleError> {
    if bytes.len() > 65536 {
        return Err(error(ErrorKind::InvalidConfig, "Windows tweak field token"));
    }
    serde_json::from_slice(bytes)
        .map_err(|_| error(ErrorKind::InvalidConfig, "Windows tweak field token"))
}
pub(super) fn toggle_label(value: StoredDword) -> &'static str {
    match value {
        StoredDword::Absent => "No explicit preference",
        StoredDword::Value(0) => "Off",
        StoredDword::Value(1) => "On",
        StoredDword::Value(_) => "Unknown",
    }
}
pub(super) fn preference_label(value: Option<&str>, gpu: bool) -> &'static str {
    match (value, gpu) {
        (None, _) => "No explicit preference",
        (Some("0"), true) => "Windows decides",
        (Some("1"), true) => "Power saving",
        (Some("2"), true) => "High performance",
        (Some("0"), false) => "Off",
        (Some("1"), false) => "On",
        _ => "Unknown",
    }
}

pub(super) fn settings_action(action: ModuleAction, page: SettingsPage) -> Result<(), ModuleError> {
    match action {
        ModuleAction::Refresh => Ok(()),
        ModuleAction::OpenSettings => gaming_diagnostics::open_settings(page).map_err(|e| {
            error(ErrorKind::Unsupported, "open Settings")
                .with_detail(e.to_string())
                .unwrap()
        }),
        _ => Err(error(ErrorKind::Unsupported, "Windows tweak action")),
    }
}
