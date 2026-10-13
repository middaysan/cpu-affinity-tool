use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
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
    fn tweak(&self) -> Option<crate::app::features::system_tweaks::presentation::TweakSpec> {
        use crate::app::features::system_tweaks::presentation::*;
        Some(TweakSpec {order:6,section:TweakSection::Comfort,title:"Mouse acceleration",scope:"Windows pointer · except raw input",apply_label:"Turn off",help:"Turn off Windows pointer acceleration for consistent movement. Games using raw mouse input usually ignore this setting. This is a comfort choice, not an FPS improvement. Restore brings back the exact pointer parameters saved before this change. High mouse polling rates can add CPU load; adjust those in your mouse software if needed."})
    }
    fn tweak_config(&self, target: &TargetIdentity) -> Result<ModuleEnvelope, ModuleError> {
        let value = gaming::read_mouse(&user(target)?).map_err(|e| native("read", e))?;
        Ok(envelope(
            ID,
            &Config {
                threshold1: value.0[0],
                threshold2: value.0[1],
                acceleration: 0,
            },
        ))
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        gaming::read_mouse(&user(target)?)
            .map(|value| value.0[2] == 0)
            .map_err(|e| native("read", e))
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let bytes = &fields.iter().find(|(id, _)| id.as_str() == FIELD)?.1;
        if bytes.len() != 12 {
            return None;
        }
        let acceleration = i32::from_le_bytes(bytes[8..12].try_into().ok()?);
        Some(if acceleration == 0 { "Off" } else { "On" }.into())
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
    #[cfg(test)]
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
