//! Manual background capture adapter; native work belongs to the application worker.
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
    windows_tweaks::{self as native_api, StoredDword},
};

const ID: &str = "background_capture";
const FIELD: &str = "background_capture_preference";
pub(crate) struct BackgroundCapture;
fn spec() -> TweakSpec {
    TweakSpec { order: 30, section: TweakSection::Games, title: "Background clips", scope: "Your Windows account", apply_label: "Turn off", help: "Turns off only the Record what happened background buffer. Game Bar, screenshots and manual recording remain available. This reads the stored preference, not recording activity. Restart a running game and verify the Captures switch in Windows Settings." }
}
impl ModuleEditor for BackgroundCapture {
    fn tweak(&self) -> Option<TweakSpec> {
        Some(spec())
    }
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WIN10_11
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, envelope: &ModuleEnvelope) -> bool {
        config(ID, envelope, false).is_ok()
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, envelope: &mut ModuleEnvelope) -> Result<bool, ()> {
        config(ID, envelope, false).map_err(|_| ())?;
        ui.label(spec().help);
        Ok(false)
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        native_api::read_background_capture(&user(target)?)
            .map(|stored| stored == StoredDword::Value(0))
            .map_err(|e| native("read", e))
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let bytes = &fields.iter().find(|(id, _)| id == &resource(FIELD))?.1;
        Some(toggle_label(value::<StoredDword>(bytes).ok()?).into())
    }
}
impl Module for BackgroundCapture {
    fn descriptor(&self) -> ModuleDescriptor {
        let spec = spec();
        let mut descriptor = descriptor(
            ID,
            spec.title,
            spec.help,
            TargetScope::User,
            vec![ActivationMode::Manual],
        );
        descriptor.effect = Effect::GameRestartRequired;
        descriptor
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        user_targets()
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        let result = (|| {
            let stored = native_api::read_background_capture(&observed_user(target)?)
                .map_err(|e| native("read", e))?;
            Ok(format!(
                "{} · background buffer preference",
                toggle_label(stored)
            ))
        })();
        let mut result = observation(result);
        result.effect = Effect::GameRestartRequired;
        result.summary.effect = Effect::GameRestartRequired;
        result
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        envelope: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        config(ID, envelope, false)?;
        user(target)?;
        Ok(vec![field(target, FIELD, token(&StoredDword::Value(0)))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        native_api::read_background_capture(&user(&key.target)?)
            .map(|stored| token(&stored))
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        native_api::write_background_capture(&user(&key.target)?, value(expected)?, value(desired)?)
            .map_err(|e| native("write", e))
    }
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        settings_action(action, SettingsPage::BackgroundCapture)
    }
}
