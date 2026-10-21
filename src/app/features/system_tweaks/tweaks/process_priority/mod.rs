use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, ModestPriority};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
const ID: &str = "process_priority";
pub(crate) const FIELD: &str = "process_priority_class";
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    priority: ModestPriority,
}
#[derive(Default)]
pub(crate) struct ProcessPriority {
    scheduling: Arc<super::scheduling::SchedulingHandoff>,
}
impl ProcessPriority {
    pub(crate) fn with_scheduling(scheduling: Arc<super::scheduling::SchedulingHandoff>) -> Self {
        Self { scheduling }
    }
}
pub(crate) fn configured_priority(
    config: &ModuleEnvelope,
) -> Result<os_api::PriorityClass, ModuleError> {
    let config: Config = decode(ID, config)?;
    Ok(match config.priority {
        ModestPriority::Idle => os_api::PriorityClass::Idle,
        ModestPriority::BelowNormal => os_api::PriorityClass::BelowNormal,
        ModestPriority::Normal => os_api::PriorityClass::Normal,
        ModestPriority::AboveNormal => os_api::PriorityClass::AboveNormal,
        ModestPriority::High => os_api::PriorityClass::High,
    })
}
#[cfg(test)]
fn label(priority: ModestPriority) -> &'static str {
    match priority {
        ModestPriority::Idle => "Idle",
        ModestPriority::BelowNormal => "Below normal",
        ModestPriority::Normal => "Normal",
        ModestPriority::AboveNormal => "Above normal",
        ModestPriority::High => "High",
    }
}
fn raw_label(priority: u32) -> &'static str {
    match priority {
        0x40 => "Idle",
        0x4000 => "Below normal",
        0x20 => "Normal",
        0x8000 => "Above normal",
        0x80 => "High",
        0x100 => "Realtime (pre-existing)",
        _ => "Unknown priority",
    }
}
impl ModuleEditor for ProcessPriority {
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        configured_priority(config).is_ok()
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value: Config = decode(ID, config).map_err(|_| ())?;
        let before = value.priority;
        eframe::egui::ComboBox::from_id_salt(ID)
            .selected_text(label(value.priority))
            .show_ui(ui, |ui| {
                for priority in [
                    ModestPriority::Idle,
                    ModestPriority::BelowNormal,
                    ModestPriority::Normal,
                    ModestPriority::AboveNormal,
                    ModestPriority::High,
                ] {
                    ui.selectable_value(&mut value.priority, priority, label(priority));
                }
            });
        ui.small("Applies to games linked to this profile. Higher priority gives the game preference when apps compete for CPU time. High may make other apps less responsive.");
        let changed = before != value.priority;
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
}
impl Module for ProcessPriority {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Game priority",
            "Choose the game's priority when apps compete for CPU time.",
            TargetScope::Process,
            automatic_modes(),
        )
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![]
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        observation((|| {
            let value = gaming::read_priority(&process(target)?).map_err(|e| native("read", e))?;
            Ok(format!("Observed priority: {}", raw_label(value)))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let process = process(target)?;
        self.scheduling
            .baseline(process.pid, process.creation_token, false)?;
        let config: Config = decode(ID, config)?;
        Ok(vec![field(
            target,
            FIELD,
            config.priority.raw().to_le_bytes().to_vec(),
        )])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        gaming::read_priority(&process(&key.target)?)
            .map(|value| value.to_le_bytes().to_vec())
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        gaming::write_priority(
            &process(&key.target)?,
            u32_value(expected)?,
            u32_value(desired)?,
        )
        .map_err(|e| native("write", e))
    }
}
