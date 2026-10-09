use super::common::*;
use crate::app::features::tuning::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::{ModuleEditor, ModuleSection},
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
    fn section(&self) -> ModuleSection {
        ModuleSection::Power
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
    fn summary(&self, config: &ModuleEnvelope) -> Option<String> {
        decode::<Config>(ID, config)
            .ok()
            .map(|c| format!("Plugged in: {}; On battery: {}", c.ac.label(), c.dc.label()))
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
        user_targets()
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation((|| {
            let user = observed_user(target)?;
            let ac =
                gaming::read_power_mode(&user, PowerSource::Ac).map_err(|e| native("read", e))?;
            let dc =
                gaming::read_power_mode(&user, PowerSource::Dc).map_err(|e| native("read", e))?;
            Ok(format!(
                "Plugged in: {}; On battery: {}. The active mode may differ.",
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
        user(target)?;
        let config: Config = decode(ID, config)?;
        Ok(vec![
            field(target, "user_power_mode_ac", config.ac.guid().0.to_vec()),
            field(target, "user_power_mode_dc", config.dc.guid().0.to_vec()),
        ])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        gaming::read_power_mode(&user(&key.target)?, source(key)?)
            .map(|v| v.0.to_vec())
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        gaming::write_power_mode(
            &user(&key.target)?,
            source(key)?,
            guid(expected)?,
            guid(desired)?,
        )
        .map_err(|e| native("write", e))
    }
}
