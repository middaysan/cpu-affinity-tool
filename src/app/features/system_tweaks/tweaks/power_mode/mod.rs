use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, GuidValue, PowerSource};
use serde::{Deserialize, Serialize};
const ID: &str = "power_mode";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    Efficiency,
    #[default]
    Balanced,
    Performance,
}
impl Mode {
    fn guid(self) -> GuidValue {
        match self {
            Self::Efficiency => gaming::POWER_MODE_EFFICIENCY,
            Self::Balanced => gaming::POWER_MODE_BALANCED,
            Self::Performance => gaming::POWER_MODE_PERFORMANCE,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Efficiency => "Best power efficiency",
            Self::Balanced => "Balanced",
            Self::Performance => "Best performance",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    ac: Mode,
    dc: Mode,
    #[serde(default)]
    ac_only: bool,
}
pub(crate) struct PowerMode;
fn source(key: &ResourceKey) -> Result<PowerSource, ModuleError> {
    if key.field == resource("user_power_mode_ac") {
        Ok(PowerSource::Ac)
    } else if key.field == resource("user_power_mode_dc") {
        Ok(PowerSource::Dc)
    } else {
        Err(error(ErrorKind::Unsupported, "power mode field"))
    }
}
fn name(guid: GuidValue) -> String {
    [Mode::Efficiency, Mode::Balanced, Mode::Performance]
        .into_iter()
        .find(|mode| mode.guid() == guid)
        .map(|m| m.label().into())
        .unwrap_or_else(|| format!("Unknown mode {:032x}", u128::from_be_bytes(guid.0)))
}
impl ModuleEditor for PowerMode {
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WINDOWS11
    }
    fn tweak(&self) -> Option<crate::app::features::system_tweaks::presentation::TweakSpec> {
        use crate::app::features::system_tweaks::presentation::*;
        Some(TweakSpec {order:0, section:TweakSection::Games, title:"Performance on AC power",scope:"This PC · plugged in only",apply_label:"Apply",help:"Prefer performance while plugged in. Battery preferences are unchanged. Useful when a power-saving preference limits the game; higher temperature, fan noise and power use are possible. Thermal and firmware limits still apply. Does not force fixed clocks or guarantee more FPS. This manual change stays until Restore; game launch and closure do not switch it."})
    }
    fn tweak_config(&self, _: &TargetIdentity) -> Result<ModuleEnvelope, ModuleError> {
        Ok(envelope(
            ID,
            &Config {
                ac: Mode::Performance,
                dc: Mode::Balanced,
                ac_only: true,
            },
        ))
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        gaming::read_power_mode(&account_user(target)?, PowerSource::Ac)
            .map(|v| v == gaming::POWER_MODE_PERFORMANCE)
            .map_err(|e| native("read", e))
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        fields
            .iter()
            .find(|(id, _)| id.as_str() == "user_power_mode_ac")
            .and_then(|(_, bytes)| guid(bytes).ok())
            .map(name)
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        decode::<Config>(ID, config).is_ok()
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value: Config = decode(ID, config).map_err(|_| ())?;
        let before = (value.ac, value.dc);
        for (id, title, mode) in [
            ("power_mode_ac", "Plugged in", &mut value.ac),
            ("power_mode_dc", "On battery", &mut value.dc),
        ] {
            ui.horizontal(|ui| {
                ui.label(title);
                eframe::egui::ComboBox::from_id_salt(id)
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for candidate in [Mode::Efficiency, Mode::Balanced, Mode::Performance] {
                            ui.selectable_value(mode, candidate, candidate.label());
                        }
                    });
            });
        }
        ui.small(
            "Choose separately for plugged-in and battery use. Windows may adjust the active mode.",
        );
        let changed = before != (value.ac, value.dc);
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
}
impl Module for PowerMode {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Power mode",
            "Balance power use and performance for plugged-in and battery use.",
            TargetScope::User,
            all_modes(),
        )
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        account_targets()
    }
    fn canonical_target(&self, target: &TargetIdentity) -> Result<TargetIdentity, ModuleError> {
        canonical_account_target(target)
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation((|| {
            let user = observed_account_user(target)?;
            let ac =
                gaming::read_power_mode(&user, PowerSource::Ac).map_err(|e| native("read", e))?;
            let dc =
                gaming::read_power_mode(&user, PowerSource::Dc).map_err(|e| native("read", e))?;
            Ok(format!(
                "{} on AC power\nBattery: {}. The active mode may differ.",
                name(ac),
                name(dc)
            ))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        account_user(target)?;
        let config: Config = decode(ID, config)?;
        let mut fields = vec![field(
            target,
            "user_power_mode_ac",
            config.ac.guid().0.to_vec(),
        )];
        if !config.ac_only {
            fields.push(field(
                target,
                "user_power_mode_dc",
                config.dc.guid().0.to_vec(),
            ));
        }
        Ok(fields)
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        gaming::read_power_mode(&account_user(&key.target)?, source(key)?)
            .map(|v| v.0.to_vec())
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        gaming::write_power_mode(
            &account_user(&key.target)?,
            source(key)?,
            guid(expected)?,
            guid(desired)?,
        )
        .map_err(|e| native("write", e))
    }
}
