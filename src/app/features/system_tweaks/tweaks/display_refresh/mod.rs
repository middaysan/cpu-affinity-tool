//! Manual screen refresh rate adapter; native work belongs to the application worker.
use super::{common::*, preference_common::*};
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::{ModuleAction, ModuleEditor, TweakSection, TweakSpec},
};
use os_api::{
    gaming_diagnostics::SettingsPage,
    windows_tweaks::{self as native_api, DisplayMode, DisplayTarget},
};
const ID: &str = "display_refresh";
const FIELD: &str = "display_refresh_mode";
fn read_card(target: &TargetIdentity) -> Result<(String, bool), ModuleError> {
    let target = display(target)
        .or_else(|_| native_api::primary_display_target().map_err(|e| native("read", e)))?;
    let modes = native_api::display_modes(&target).map_err(|e| native("read", e))?;
    let highest = native_api::highest_compatible_mode(modes.current, &modes.compatible);
    Ok((
        format!(
            "{} Hz · highest {} Hz",
            modes.current.frequency,
            highest.map_or(modes.current.frequency, |m| m.frequency)
        ),
        highest == Some(modes.current),
    ))
}
fn restorable_baseline(modes: &native_api::DisplayModes) -> Result<(), ModuleError> {
    if !modes.compatible.contains(&modes.current) {
        return Err(error(
            ErrorKind::Unsupported,
            "The current refresh mode cannot be verified for Undo",
        ));
    }
    Ok(())
}
pub(crate) struct DisplayRefresh;
fn display(target: &TargetIdentity) -> Result<DisplayTarget, ModuleError> {
    let TargetIdentity::Device { identity } = target else {
        return Err(error(ErrorKind::Unsupported, "display target"));
    };
    if !target.valid() {
        return Err(error(ErrorKind::Unsupported, "display target"));
    }
    let identity = identity
        .strip_prefix("display:")
        .ok_or_else(|| error(ErrorKind::Unsupported, "display target"))?;
    let value: DisplayTarget = serde_json::from_str(identity)
        .map_err(|_| error(ErrorKind::InvalidConfig, "display identity"))?;
    if !(TargetIdentity::User {
        sid: value.user_sid.clone(),
        logon: value.user_logon,
        session: value.user_session,
    })
    .valid()
        || value.device_name.is_empty()
        || value.adapter_id.is_empty()
        || value.monitor_id.is_empty()
        || [&value.device_name, &value.adapter_id, &value.monitor_id]
            .iter()
            .any(|s| s.contains('\0'))
    {
        return Err(error(ErrorKind::InvalidConfig, "display identity"));
    }
    Ok(value)
}
fn display_target(value: &DisplayTarget) -> Result<TargetIdentity, ModuleError> {
    let target = TargetIdentity::Device {
        identity: "display:".to_owned()
            + &serde_json::to_string(value)
                .map_err(|_| error(ErrorKind::InvalidConfig, "display identity"))?,
    };
    display(&target)?;
    Ok(target)
}
fn spec() -> TweakSpec {
    TweakSpec { order: 50, section: TweakSection::Games, title: "Screen refresh rate", scope: "Current main screen", apply_label: "Use highest Hz", help: "Tests the highest reported refresh mode at the current resolution, 32-bit color, orientation and scan mode, then applies a temporary desktop change. Confirm within 15 seconds or the app attempts exact undo. Resolution and other screens are preserved. Nominal screen Hz is not game FPS; HDR/link format, dynamic/variable refresh and fractional timings are not measured." }
}
impl ModuleEditor for DisplayRefresh {
    fn tweak(&self) -> Option<TweakSpec> {
        Some(spec())
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, envelope: &ModuleEnvelope) -> bool {
        config(ID, envelope, true).is_ok()
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, envelope: &mut ModuleEnvelope) -> Result<bool, ()> {
        config(ID, envelope, true).map_err(|_| ())?;
        ui.label(spec().help);
        Ok(false)
    }
    fn tweak_config(&self, target: &TargetIdentity) -> Result<ModuleEnvelope, ModuleError> {
        let modes = native_api::display_modes(&display(target)?).map_err(|e| native("read", e))?;
        restorable_baseline(&modes)?;
        let refresh = native_api::highest_compatible_mode(modes.current, &modes.compatible)
            .ok_or_else(|| error(ErrorKind::Unsupported, "no compatible refresh mode"))?;
        Ok(envelope(
            ID,
            &Config {
                recommended: true,
                refresh: Some(refresh),
            },
        ))
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        let modes = native_api::display_modes(&display(target)?).map_err(|e| native("read", e))?;
        Ok(
            native_api::highest_compatible_mode(modes.current, &modes.compatible)
                .is_some_and(|mode| mode == modes.current),
        )
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let bytes = &fields.iter().find(|(id, _)| id == &resource(FIELD))?.1;
        let mode: DisplayMode = value(bytes).ok()?;
        Some(format!(
            "{} Hz · {} × {}",
            mode.frequency, mode.width, mode.height
        ))
    }
}

#[cfg(test)]
mod baseline_tests {
    use super::*;
    #[test]
    fn display_refresh_acquisition_requires_tested_original_mode() {
        let current = DisplayMode {
            width: 1920,
            height: 1080,
            bits_per_pixel: 32,
            frequency: 75,
            flags: 0,
            orientation: 0,
            fixed_output: 0,
        };
        let high = DisplayMode {
            frequency: 144,
            ..current
        };
        let mut modes = native_api::DisplayModes {
            current,
            compatible: vec![high],
        };
        assert!(restorable_baseline(&modes).is_err());
        modes.compatible.push(current);
        assert!(restorable_baseline(&modes).is_ok());
    }
}
impl Module for DisplayRefresh {
    fn target_has_expired(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        os_api::gaming::session_has_ended(&display(target)?.user())
            .map_err(|e| native("session lifetime", e))
    }
    fn descriptor(&self) -> ModuleDescriptor {
        let spec = spec();
        let mut descriptor = descriptor(
            ID,
            spec.title,
            spec.help,
            TargetScope::PnpDevice,
            vec![ActivationMode::Manual],
        );
        descriptor.target_scopes.push(TargetScope::Display);
        descriptor
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        native_api::primary_display_target()
            .ok()
            .and_then(|target| display_target(&target).ok())
            .into_iter()
            .collect()
    }
    fn accepts_target(&self, target: &TargetIdentity) -> bool {
        display(target).is_ok_and(|target| native_api::display_modes(&target).is_ok())
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation(read_card(target).map(|(label, _)| label))
    }
    fn observe_tweak(
        &self,
        target: &TargetIdentity,
        _: Option<&ModuleEnvelope>,
    ) -> (
        ModuleObservation,
        Result<bool, ModuleError>,
        Option<ModuleEnvelope>,
    ) {
        let result = read_card(target);
        (
            observation(result.clone().map(|(label, _)| label)),
            result.map(|(_, ready)| ready),
            None,
        )
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        envelope: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let config = config(ID, envelope, true)?;
        let modes = native_api::display_modes(&display(target)?).map_err(|e| native("read", e))?;
        restorable_baseline(&modes)?;
        let refresh = config
            .refresh
            .ok_or_else(|| error(ErrorKind::InvalidConfig, "Select a verified refresh mode"))?;
        Ok(vec![field(target, FIELD, token(&refresh))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        native_api::display_modes(&display(&key.target)?)
            .map(|value| token(&value.current))
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        native_api::write_display_mode(&display(&key.target)?, value(expected)?, value(desired)?)
            .map_err(|e| native("write", e))
    }
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        settings_action(action, SettingsPage::Display)
    }
}
