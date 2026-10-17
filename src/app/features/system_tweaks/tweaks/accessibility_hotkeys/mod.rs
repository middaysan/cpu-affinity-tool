use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, AccessibilityFeature, AccessibilityState};
use serde::{Deserialize, Serialize};
const ID: &str = "accessibility_hotkeys";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub sticky: bool,
    pub filter: bool,
    pub toggle: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            sticky: true,
            filter: true,
            toggle: true,
        }
    }
}
pub(crate) struct AccessibilityHotkeys;
fn resource_name(feature: AccessibilityFeature) -> &'static str {
    match feature {
        AccessibilityFeature::Sticky => "sticky_hotkey_flags",
        AccessibilityFeature::Filter => "filter_hotkey_flags",
        AccessibilityFeature::Toggle => "toggle_hotkey_flags",
    }
}
fn key_feature(key: &ResourceKey) -> Result<AccessibilityFeature, ModuleError> {
    [
        AccessibilityFeature::Sticky,
        AccessibilityFeature::Filter,
        AccessibilityFeature::Toggle,
    ]
    .into_iter()
    .find(|f| key.field == resource(resource_name(*f)))
    .ok_or_else(|| error(ErrorKind::Unsupported, "accessibility field"))
}
pub(super) fn changes(
    states: [(AccessibilityFeature, AccessibilityState); 3],
    config: &Config,
) -> Vec<(AccessibilityFeature, u32)> {
    states
        .into_iter()
        .filter(|(feature, state)| {
            !state.enabled
                && match feature {
                    AccessibilityFeature::Sticky => config.sticky,
                    AccessibilityFeature::Filter => config.filter,
                    AccessibilityFeature::Toggle => config.toggle,
                }
        })
        .map(|(feature, _)| (feature, 0))
        .collect()
}
impl ModuleEditor for AccessibilityHotkeys {
    fn tweak(&self) -> Option<crate::app::features::system_tweaks::presentation::TweakSpec> {
        use crate::app::features::system_tweaks::presentation::*;
        Some(TweakSpec {order:5,section:TweakSection::Comfort,title:"Shift and NumLock popups",scope:"Activation shortcuts · your account",apply_label:"Turn off",help:"Suppress activation shortcuts for inactive Sticky, Filter and Toggle Keys so accidental key presses do not interrupt play. Features already in use and their shortcuts are preserved. Other Windows shortcuts are unchanged. This is a comfort choice without an FPS gain. Restore returns only the shortcut flags this app changed."})
    }
    fn tweak_ready(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        let user = user(target)?;
        for feature in [
            AccessibilityFeature::Sticky,
            AccessibilityFeature::Filter,
            AccessibilityFeature::Toggle,
        ] {
            let state =
                gaming::read_accessibility(&user, feature).map_err(|e| native("read", e))?;
            if !state.enabled && state.hotkey_flags != 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
        let labels: Vec<_> = fields
            .iter()
            .filter_map(|(id, bytes)| {
                let value = u32::from_le_bytes(bytes.as_slice().try_into().ok()?);
                let name = match id.as_str() {
                    "sticky_hotkey_flags" => "Sticky",
                    "filter_hotkey_flags" => "Filter",
                    "toggle_hotkey_flags" => "Toggle",
                    _ => return None,
                };
                Some(format!("{name}: {}", if value == 0 { "Off" } else { "On" }))
            })
            .collect();
        (!labels.is_empty()).then(|| labels.join(" · "))
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
        let before = value.clone();
        ui.checkbox(&mut value.sticky, "Block Sticky Keys shortcut");
        ui.checkbox(&mut value.filter, "Block Filter Keys shortcut");
        ui.checkbox(&mut value.toggle, "Block Toggle Keys shortcut");
        ui.small(
            "Already enabled features are skipped. Other accessibility settings stay the same.",
        );
        let changed = before != value;
        if changed {
            *config = envelope(ID, &value);
        }
        Ok(changed)
    }
}
impl Module for AccessibilityHotkeys {
    fn target_has_expired(&self, target: &TargetIdentity) -> Result<bool, ModuleError> {
        gaming::session_has_ended(&user(target)?).map_err(|e| native("session lifetime", e))
    }
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Accessibility shortcuts",
            "Keep Sticky, Filter and Toggle Keys shortcuts from interrupting play.",
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
            let user = observed_user(target)?;
            let mut summaries = Vec::new();
            for (feature, name) in [
                (AccessibilityFeature::Sticky, "Sticky"),
                (AccessibilityFeature::Filter, "Filter"),
                (AccessibilityFeature::Toggle, "Toggle"),
            ] {
                let value =
                    gaming::read_accessibility(&user, feature).map_err(|e| native("read", e))?;
                summaries.push(format!(
                    "{name}: {}",
                    if value.enabled {
                        "feature enabled; skipped"
                    } else if value.hotkey_flags & 4 != 0 {
                        "hotkey enabled"
                    } else {
                        "hotkey disabled"
                    }
                ));
            }
            Ok(summaries.join("; "))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let user = user(target)?;
        let config: Config = decode(ID, config)?;
        let features = [
            AccessibilityFeature::Sticky,
            AccessibilityFeature::Filter,
            AccessibilityFeature::Toggle,
        ];
        let mut states = [(
            AccessibilityFeature::Sticky,
            AccessibilityState {
                enabled: false,
                hotkey_flags: 0,
            },
        ); 3];
        for (index, feature) in features.into_iter().enumerate() {
            states[index] = (
                feature,
                gaming::read_accessibility(&user, feature).map_err(|e| native("read", e))?,
            );
        }
        Ok(changes(states, &config)
            .into_iter()
            .map(|(feature, flags)| {
                field(target, resource_name(feature), flags.to_le_bytes().to_vec())
            })
            .collect())
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        gaming::read_accessibility(&user(&key.target)?, key_feature(key)?)
            .map(|v| v.hotkey_flags.to_le_bytes().to_vec())
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        gaming::write_accessibility(
            &user(&key.target)?,
            key_feature(key)?,
            u32_value(expected)?,
            u32_value(desired)?,
        )
        .map_err(|e| native("write", e))
    }
}
