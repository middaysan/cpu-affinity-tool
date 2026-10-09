use super::common::*;
use crate::app::features::tuning::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::{ModuleEditor, ModuleSection},
};
use os_api::gaming::{self, MouseParameters};
use serde::{Deserialize, Serialize};
const ID: &str = "mouse_acceleration";
const FIELD: &str = "mouse_acceleration_tuple";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    threshold1: i32,
    threshold2: i32,
    acceleration: i32,
}
impl Config {
    fn valid(&self) -> bool {
        (0..=1024).contains(&self.threshold1)
            && (0..=1024).contains(&self.threshold2)
            && (0..=2).contains(&self.acceleration)
    }
    fn value(&self) -> MouseParameters {
        MouseParameters([self.threshold1, self.threshold2, self.acceleration])
    }
}
pub(crate) struct MouseAcceleration;
impl ModuleEditor for MouseAcceleration {
    fn section(&self) -> ModuleSection {
        ModuleSection::Input
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        decode::<Config>(ID, config).is_ok_and(|v| v.valid())
    }
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let mut value: Config = decode(ID, config).map_err(|_| ())?;
        if !value.valid() {
            return Err(());
        }
        let before = value.clone();
        eframe::egui::ComboBox::from_id_salt(ID)
            .selected_text(match value.acceleration {
                0 => "Off · consistent movement",
                1 => "On · one speed threshold",
                _ => "On · two speed thresholds",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut value.acceleration, 0, "Off · consistent movement");
                ui.selectable_value(&mut value.acceleration, 1, "On · one speed threshold");
                ui.selectable_value(&mut value.acceleration, 2, "On · two speed thresholds");
            });
        if value.acceleration != 0 {
            ui.horizontal(|ui| {
                ui.label("Speed thresholds");
                ui.add(eframe::egui::DragValue::new(&mut value.threshold1).range(0..=1024));
                ui.add(eframe::egui::DragValue::new(&mut value.threshold2).range(0..=1024));
            });
        }
        ui.small("With acceleration on, faster mouse movement moves the pointer farther. Games using raw mouse input may ignore this setting.");
        let changed = before != value;
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
    fn summary(&self, config: &ModuleEnvelope) -> Option<String> {
        decode::<Config>(ID, config).ok().map(|c| {
            if c.acceleration == 0 {
                "Acceleration off".into()
            } else {
                format!(
                    "Acceleration level {} · speed thresholds {} / {}",
                    c.acceleration, c.threshold1, c.threshold2
                )
            }
        })
    }
}
impl Module for MouseAcceleration {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Mouse acceleration",
            "Choose how mouse movement affects the Windows pointer.",
            TargetScope::User,
            vec![ActivationMode::Manual, ActivationMode::ForegroundSession],
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
            let value =
                gaming::read_mouse(&observed_user(target)?).map_err(|e| native("read", e))?;
            Ok(format!(
                "Thresholds {} / {}; acceleration {}",
                value.0[0], value.0[1], value.0[2]
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
        if !config.valid() {
            return Err(error(ErrorKind::InvalidConfig, "mouse configuration"));
        }
        Ok(vec![field(target, FIELD, mouse_bytes(config.value()))])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        gaming::read_mouse(&user(&key.target)?)
            .map(mouse_bytes)
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        gaming::write_mouse(&user(&key.target)?, mouse(expected)?, mouse(desired)?)
            .map_err(|e| native("write", e))
    }
}
