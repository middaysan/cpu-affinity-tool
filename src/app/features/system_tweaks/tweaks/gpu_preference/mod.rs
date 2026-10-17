//! Manual game GPU preference adapter; native work belongs to the application worker.
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
    windows_tweaks::{self as native_api, GpuTarget},
};
use std::path::PathBuf;
const ID: &str = "gpu_preference";
const FIELD: &str = "gpu_preference_field";
pub(crate) struct GpuPreference;
fn gpu(target: &TargetIdentity) -> Result<GpuTarget, ModuleError> {
    let TargetIdentity::Device { identity } = target else {
        return Err(error(ErrorKind::Unsupported, "GPU executable target"));
    };
    if !target.valid() {
        return Err(error(ErrorKind::Unsupported, "GPU executable target"));
    }
    let raw = identity
        .strip_prefix("gpu_exe:")
        .ok_or_else(|| error(ErrorKind::Unsupported, "GPU executable target"))?;
    let value: GpuTarget = serde_json::from_str(raw)
        .map_err(|_| error(ErrorKind::InvalidConfig, "GPU executable identity"))?;
    if !(TargetIdentity::Account {
        sid: value.user_sid.clone(),
    })
    .valid()
        || value.executable.is_empty()
        || value.executable.len() > 32767
        || value.executable.contains('\0')
    {
        return Err(error(ErrorKind::InvalidConfig, "GPU executable identity"));
    }
    Ok(value)
}
fn spec() -> TweakSpec {
    TweakSpec { order: 20, section: TweakSection::Hardware, title: "Game GPU", scope: "Selected saved EXE", apply_label: "High performance", help: "Sets the high-performance GPU preference for one existing saved executable. Other graphics fields are preserved. It is useful when Windows offers multiple suitable GPUs; the preference does not prove which GPU the game uses. Restart the game and inspect its renderer. Vendor latency and shader controls remain in vendor or game settings." }
}
impl ModuleEditor for GpuPreference {
    fn tweak(&self) -> Option<TweakSpec> {
        Some(spec())
    }
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::GPU_PREFERENCE
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
    fn selects_saved_exe(&self) -> bool {
        true
    }
    fn qualify_tweak_target(&self, target: &TargetIdentity) -> Result<TargetIdentity, ModuleError> {
        if gpu(target).is_ok() {
            let canonical = self.canonical_target(target)?;
            let qualified = gpu(&canonical)?;
            let actual = native_api::qualify_gpu_target(&qualified.executable)
                .map_err(|e| native("qualify GPU executable", e))?;
            if actual != qualified {
                return Err(error(ErrorKind::ExternalChange, "GPU executable identity"));
            }
            return Ok(canonical);
        }
        let TargetIdentity::Device { identity } = target else {
            return Err(error(ErrorKind::Unsupported, "GPU executable target"));
        };
        let path = identity
            .strip_prefix("gpu_exe:")
            .ok_or_else(|| error(ErrorKind::Unsupported, "GPU executable target"))?;
        let qualified = native_api::qualify_gpu_target(path)
            .map_err(|e| native("qualify GPU executable", e))?;
        let target = TargetIdentity::Device {
            identity: "gpu_exe:".to_owned()
                + &serde_json::to_string(&qualified)
                    .map_err(|_| error(ErrorKind::InvalidConfig, "GPU executable identity"))?,
        };
        gpu(&target)?;
        Ok(target)
    }
    fn tweak_executable(&self, target: &TargetIdentity) -> Option<PathBuf> {
        gpu(target).ok().map(|gpu| PathBuf::from(gpu.executable))
    }

    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        let gpu = gpu(target)?;
        native_api::read_gpu_preference(&gpu.user(), &gpu.executable)
            .map(|value| value.as_deref() == Some("2"))
            .map_err(|e| native("read", e))
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let bytes = &fields.iter().find(|(id, _)| id == &resource(FIELD))?.1;
        Some(preference_label(value::<Option<String>>(bytes).ok()?.as_deref(), true).into())
    }
}
impl Module for GpuPreference {
    fn descriptor(&self) -> ModuleDescriptor {
        let spec = spec();
        let mut descriptor = descriptor(
            ID,
            spec.title,
            spec.help,
            TargetScope::PnpDevice,
            vec![ActivationMode::Manual],
        );
        descriptor.target_scopes.push(TargetScope::Gpu);
        descriptor.effect = Effect::GameRestartRequired;
        descriptor
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        Vec::new()
    }
    fn canonical_target(&self, target: &TargetIdentity) -> Result<TargetIdentity, ModuleError> {
        let mut qualified = gpu(target)?;
        qualified.user_logon = 0;
        let canonical = TargetIdentity::Device {
            identity: "gpu_exe:".to_owned()
                + &serde_json::to_string(&qualified)
                    .map_err(|_| error(ErrorKind::InvalidConfig, "GPU executable identity"))?,
        };
        gpu(&canonical)?;
        Ok(canonical)
    }
    fn accepts_target(&self, target: &TargetIdentity) -> bool {
        let Ok(canonical) = self.canonical_target(target) else {
            return false;
        };
        gpu(&canonical).is_ok_and(|target| {
            native_api::qualify_gpu_target(&target.executable).is_ok_and(|actual| actual == target)
        })
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        let result = (|| {
            let gpu = gpu(target)
                .map_err(|_| error(ErrorKind::Unsupported, "Choose a saved executable"))?;
            Ok(format!(
                "{} · stored preference",
                preference_label(
                    native_api::read_gpu_preference(&gpu.user(), &gpu.executable)
                        .map_err(|e| native("read", e))?
                        .as_deref(),
                    true
                )
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
        gpu(target)?;
        Ok(vec![field(target, FIELD, token(&Some("2".to_owned())))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        let gpu = gpu(&key.target)?;
        native_api::read_gpu_preference(&gpu.user(), &gpu.executable)
            .map(|value| token(&value))
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        let gpu = gpu(&key.target)?;
        native_api::write_gpu_preference(
            &gpu.user(),
            &gpu.executable,
            value::<Option<String>>(expected)?.as_deref(),
            value::<Option<String>>(desired)?.as_deref(),
        )
        .map_err(|e| native("write", e))
    }
    fn action(&self, action: ModuleAction) -> Result<(), ModuleError> {
        settings_action(action, SettingsPage::GpuPreference)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_account_gpu_targets_drop_legacy_logon_but_keep_sid_and_executable() {
        let target = |sid: &str, logon: u64, executable: &str| TargetIdentity::Device {
            identity: "gpu_exe:".to_owned()
                + &serde_json::to_string(&GpuTarget {
                    user_sid: sid.into(),
                    user_logon: logon,
                    executable: executable.into(),
                })
                .unwrap(),
        };
        let old = target("S-1-5-21-1", 77, r"C:\game.exe");
        let current = target("S-1-5-21-1", 78, r"C:\game.exe");
        let canonical = GpuPreference.canonical_target(&old).unwrap();
        assert_eq!(GpuPreference.canonical_target(&current).unwrap(), canonical);
        assert_eq!(
            GpuPreference.canonical_target(&canonical).unwrap(),
            canonical
        );
        let TargetIdentity::Device { identity } = &canonical else {
            panic!()
        };
        assert!(!identity.contains("user_logon"));
        assert_eq!(gpu(&canonical).unwrap().executable, r"C:\game.exe");
        assert_eq!(gpu(&canonical).unwrap().user_sid, "S-1-5-21-1");
        for distinct in [
            target("S-1-5-21-2", 78, r"C:\game.exe"),
            target("S-1-5-21-1", 78, r"C:\another.exe"),
        ] {
            assert_ne!(
                GpuPreference.canonical_target(&distinct).unwrap(),
                canonical
            );
        }
        assert!(GpuPreference
            .canonical_target(&target("S-1-x", 77, r"C:\game.exe"))
            .is_err());
    }

    #[test]
    fn windows_tweaks_gpu_target_requires_durable_user_identity() {
        let target = TargetIdentity::Device {
            identity: r"gpu_exe:C:\game.exe".into(),
        };
        assert!(gpu(&target).is_err());
        let target = TargetIdentity::Device {
            identity: "gpu_exe:".to_owned()
                + &serde_json::to_string(&GpuTarget {
                    user_sid: "S-1-5-21-1".into(),
                    user_logon: 77,
                    executable: r"C:\game.exe".into(),
                })
                .unwrap(),
        };
        assert_eq!(gpu(&target).unwrap().user_logon, 77);
    }
}
