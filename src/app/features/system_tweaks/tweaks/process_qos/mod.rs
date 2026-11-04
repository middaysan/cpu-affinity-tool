use super::common::*;
use crate::app::features::system_tweaks::{
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        registry::{FieldChange, Module, ModuleObservation},
    },
    presentation::ModuleEditor,
};
use os_api::gaming::{self, OwnedPolicy, EXECUTION_SPEED};
use serde::{Deserialize, Serialize};
const ID: &str = "process_qos";
const FIELD: &str = "process_execution_speed";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Policy {
    SystemManaged,
    EcoQos,
    #[default]
    DisableThrottling,
}
impl Policy {
    fn value(self) -> OwnedPolicy {
        match self {
            Self::SystemManaged => OwnedPolicy {
                control: 0,
                state: 0,
            },
            Self::EcoQos => OwnedPolicy {
                control: EXECUTION_SPEED,
                state: EXECUTION_SPEED,
            },
            Self::DisableThrottling => OwnedPolicy {
                control: EXECUTION_SPEED,
                state: 0,
            },
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    policy: Policy,
}
pub(crate) struct ProcessQos;
impl ModuleEditor for ProcessQos {
    fn platform_support(&self) -> crate::app::features::system_tweaks::platforms::PlatformSupport {
        crate::app::features::system_tweaks::platforms::PlatformSupport {
            windows10_min_build: Some(16299),
            windows11_min_build: Some(22000),
        }
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
}
impl Module for ProcessQos {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor(
            ID,
            "Game power saving",
            "Choose how Windows balances CPU speed and power use for this game.",
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
            let value = gaming::read_process_policy(&process(target)?, EXECUTION_SPEED)
                .map_err(|e| native("read", e))?;
            Ok(format!(
                "Game power saving: {}",
                if value.control == 0 {
                    "Windows default"
                } else if value.state != 0 {
                    "preferred"
                } else {
                    "off"
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
        let config: Config = decode(ID, config)?;
        Ok(vec![field(
            target,
            FIELD,
            policy_bytes(config.policy.value()),
        )])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        check_field(key, FIELD)?;
        gaming::read_process_policy(&process(&key.target)?, EXECUTION_SPEED)
            .map(policy_bytes)
            .map_err(|e| native("read", e))
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        check_field(key, FIELD)?;
        gaming::write_process_policy(
            &process(&key.target)?,
            EXECUTION_SPEED,
            policy(expected, EXECUTION_SPEED)?,
            policy(desired, EXECUTION_SPEED)?,
        )
        .map_err(|e| native("write", e))
    }
}
