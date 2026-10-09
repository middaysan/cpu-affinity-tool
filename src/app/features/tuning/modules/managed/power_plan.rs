use super::common::*;
use crate::app::features::tuning::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::{ModuleEditor, ModuleSection},
};
use os_api::gaming::{self, GuidValue, PowerScheme};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
const ID: &str = "power_plan";
const FIELD: &str = "active_power_scheme";
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    scheme: Option<GuidValue>,
}
#[derive(Default)]
pub(crate) struct PowerPlan {
    schemes: RwLock<Vec<PowerScheme>>,
}
impl PowerPlan {
    fn name(&self, scheme: Option<GuidValue>) -> String {
        match scheme {
            None => "Keep current plan".into(),
            Some(id) => self
                .schemes
                .read()
                .ok()
                .and_then(|schemes| schemes.iter().find(|s| s.id == id).map(|s| s.name.clone()))
                .unwrap_or_else(|| format!("Scheme {:032x}", u128::from_be_bytes(id.0))),
        }
    }
}
impl ModuleEditor for PowerPlan {
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
        let before = value.scheme;
        eframe::egui::ComboBox::from_id_salt(ID)
            .selected_text(self.name(value.scheme))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut value.scheme, None, "Keep current plan");
                if let Ok(schemes) = self.schemes.read() {
                    for scheme in schemes.iter() {
                        ui.selectable_value(&mut value.scheme, Some(scheme.id), &scheme.name);
                    }
                }
            });
        ui.small("Choose a Windows power plan. Your original plan is restored when this module is turned off.");
        let changed = before != value.scheme;
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
    fn summary(&self, config: &ModuleEnvelope) -> Option<String> {
        decode::<Config>(ID, config)
            .ok()
            .map(|c| self.name(c.scheme))
    }
}
impl Module for PowerPlan {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Power plan",
            "Choose an existing Windows power plan.",
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
            let schemes = gaming::power_schemes(&user).map_err(|e| native("read", e))?;
            let active = gaming::read_power_plan(&user).map_err(|e| native("read", e))?;
            let summary = schemes
                .iter()
                .find(|s| s.id == active)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("Scheme {:032x}", u128::from_be_bytes(active.0)));
            *self
                .schemes
                .write()
                .map_err(|_| error(ErrorKind::Read, "scheme metadata"))? = schemes;
            Ok(format!("Active scheme: {summary}"))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let config: Config = decode(ID, config)?;
        let user = user(target)?;
        let selected = match config.scheme {
            Some(id) => id,
            None => gaming::read_power_plan(&user).map_err(|e| native("read", e))?,
        };
        if !gaming::power_schemes(&user)
            .map_err(|e| native("read", e))?
            .iter()
            .any(|scheme| scheme.id == selected)
        {
            return Err(error(
                ErrorKind::Unsupported,
                "selected scheme no longer exists",
            ));
        }
        Ok(vec![field(target, FIELD, selected.0.to_vec())])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        gaming::read_power_plan(&user(&key.target)?)
            .map(|v| v.0.to_vec())
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        gaming::write_power_plan(&user(&key.target)?, guid(expected)?, guid(desired)?)
            .map_err(|e| native("write", e))
    }
}
