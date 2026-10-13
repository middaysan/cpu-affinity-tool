//! Manual windowed optimizations adapter; native work belongs to the application worker.
use super::{common::*, preference_common::*};
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::{ModuleAction, ModuleEditor, TweakSection, TweakSpec},
};
use os_api::{gaming_diagnostics::SettingsPage, windows_tweaks as native_api};

const ID: &str = "windowed_optimizations";
const FIELD: &str = "windowed_optimization_preference";
pub(crate) struct WindowedOptimizations;
fn spec() -> TweakSpec {
    TweakSpec { order: 40, section: TweakSection::Games, title: "Windowed games", scope: "Compatible DX10/11 games", apply_label: "Turn on", help: "Enables the stored Windows windowed-game optimization preference for compatible DX10/11 windowed and borderless games. Restart the game after a change. Auto HDR, VRR, GPU selection and per-app exceptions are preserved. Undo refuses to turn this off while Auto HDR depends on it." }
}
impl ModuleEditor for WindowedOptimizations {
    fn tweak(&self) -> Option<TweakSpec> {
        Some(spec())
    }
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WINDOWED_GAMES
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
        native_api::read_windowed_optimizations(&user(target)?)
            .map(|stored| stored.as_deref() == Some("1"))
            .map_err(|e| native("read", e))
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let bytes = &fields.iter().find(|(id, _)| id == &resource(FIELD))?.1;
        Some(preference_label(value::<Option<String>>(bytes).ok()?.as_deref(), false).into())
    }
}
impl Module for WindowedOptimizations {
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
            let stored = native_api::read_windowed_optimizations(&observed_user(target)?)
                .map_err(|e| native("read", e))?;
            Ok(format!(
                "{} · stored preference",
                preference_label(stored.as_deref(), false)
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
        Ok(vec![field(target, FIELD, token(&Some("1".to_owned())))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        native_api::read_windowed_optimizations(&user(&key.target)?)
            .map(|stored| token(&stored))
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        native_api::write_windowed_optimizations(
            &user(&key.target)?,
            value::<Option<String>>(expected)?.as_deref(),
            value::<Option<String>>(desired)?.as_deref(),
        )
        .map_err(|e| native("write", e))
    }
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        settings_action(action, SettingsPage::WindowedOptimizations)
    }
}
