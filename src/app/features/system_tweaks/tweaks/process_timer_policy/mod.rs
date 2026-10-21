use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, OwnedPolicy, IGNORE_TIMER_RESOLUTION};
use serde::{Deserialize, Serialize};
const ID: &str = "process_timer_policy";
const FIELD: &str = "process_ignore_timer_resolution";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    honor_existing_requests: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            honor_existing_requests: true,
        }
    }
}
pub(crate) struct ProcessTimerPolicy;
impl ModuleEditor for ProcessTimerPolicy {
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::WINDOWS11
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        envelope(ID, &Config::default())
    }
    fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
        decode::<Config>(ID, config).is_ok_and(|c| c.honor_existing_requests)
    }
    #[cfg(test)]
    fn show(&self, ui: &mut eframe::egui::Ui, config: &mut ModuleEnvelope) -> Result<bool, ()> {
        let value: Config = decode(ID, config).map_err(|_| ())?;
        if !value.honor_existing_requests {
            return Err(());
        }
        ui.label("Use the timing requests the game already makes");
        ui.small("Optional experiment. Creates no new timing requests and changes no timers for other apps. Compare results with the game visible and minimized.");
        Ok(false)
    }
}
impl Module for ProcessTimerPolicy {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Game timers · optional experiment",
            "Let a linked game use its existing timing requests.",
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
            let value = gaming::read_process_policy(&process(target)?, IGNORE_TIMER_RESOLUTION)
                .map_err(|e| native("read", e))?;
            Ok(format!(
                "Game timing requests: {}. Timing accuracy and input delay are not measured.",
                if value.control == 0 {
                    "Windows default"
                } else if value.state != 0 {
                    "ignored"
                } else {
                    "honored"
                }
            ))
        })())
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        process(target)?;
        if !self.accepts_config(config) {
            return Err(error(
                ErrorKind::InvalidConfig,
                "timer policy configuration",
            ));
        }
        Ok(vec![field(
            target,
            FIELD,
            policy_bytes(OwnedPolicy {
                control: IGNORE_TIMER_RESOLUTION,
                state: 0,
            }),
        )])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        gaming::read_process_policy(&process(&key.target)?, IGNORE_TIMER_RESOLUTION)
            .map(policy_bytes)
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        gaming::write_process_policy(
            &process(&key.target)?,
            IGNORE_TIMER_RESOLUTION,
            policy(expected, IGNORE_TIMER_RESOLUTION)?,
            policy(desired, IGNORE_TIMER_RESOLUTION)?,
        )
        .map_err(|e| native("write", e))
    }
}
