//! Manual game mode adapter; native work belongs to the application worker.
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

const ID: &str = "game_mode";
const FIELD: &str = "game_mode_preference";
pub(crate) struct GameMode;
fn spec() -> TweakSpec {
    TweakSpec { order: 20, section: TweakSection::Games, title: "Game Mode", scope: "Your Windows account", apply_label: "Turn on", help: "Enables the Windows Game Mode preference. Its benefit depends on the game and competing work. The verified value is the stored Windows preference; restart the game after changing it. No recording or Game Bar settings are changed." }
}
impl ModuleEditor for GameMode {
    fn tweak(&self) -> Option<TweakSpec> {
        Some(spec())
    }
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::GAME_MODE
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
        native_api::read_game_mode(&user(target)?)
            .map(|stored| stored == StoredDword::Value(1))
            .map_err(|e| native("read", e))
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let bytes = &fields.iter().find(|(id, _)| id == &resource(FIELD))?.1;
        Some(toggle_label(value::<StoredDword>(bytes).ok()?).into())
    }
}
impl Module for GameMode {
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
            let stored = native_api::read_game_mode(&observed_user(target)?)
                .map_err(|e| native("read", e))?;
            Ok(format!("{} · stored preference", toggle_label(stored)))
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
        Ok(vec![field(target, FIELD, token(&StoredDword::Value(1)))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        native_api::read_game_mode(&user(&key.target)?)
            .map(|stored| token(&stored))
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        native_api::write_game_mode(&user(&key.target)?, value(expected)?, value(desired)?)
            .map_err(|e| native("write", e))
    }
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        settings_action(action, SettingsPage::GameMode)
    }
}
