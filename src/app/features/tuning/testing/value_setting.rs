//! A distinct typed single-field adapter sharing the two-field native test backend.
use super::super::platform::{
    catalog::Module,
    codec::{CodecError, EnvelopeKind, JsonCodec, ModuleEnvelope, VersionedCodec},
    contract::*,
};
use super::user_setting::{value_native, value_token, NativeValue, UserModule, UserStep};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueConfig {
    pub value: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSnapshot {
    baseline: Option<NativeValue>,
    desired: Option<NativeValue>,
    known: Vec<Option<NativeValue>>,
}
pub struct ValueStep {
    before: Option<NativeValue>,
    after: Option<NativeValue>,
}
#[derive(Clone)]
pub struct ValueModule {
    backend: UserModule,
    config_codec: Arc<JsonCodec<ValueConfig>>,
    snapshot_codec: Arc<JsonCodec<ValueSnapshot>>,
}
fn key(target: &TargetIdentity) -> ResourceKey {
    ResourceKey {
        target: target.clone(),
        field: ResourceId::new("user_value").unwrap(),
    }
}
fn claim(target: &TargetIdentity, value: &Option<NativeValue>) -> ResourceClaim {
    ResourceClaim {
        key: key(target),
        desired: value_token(value),
    }
}
impl ValueModule {
    pub fn new(backend: UserModule) -> Self {
        let id = FeatureId::new("test_value_setting").unwrap();
        let version = SchemaVersion::new(1).unwrap();
        Self {
            backend,
            config_codec: Arc::new(JsonCodec::new(id.clone(), EnvelopeKind::Config, version)),
            snapshot_codec: Arc::new(JsonCodec::new(id, EnvelopeKind::Snapshot, version)),
        }
    }
    pub fn config_envelope(&self, config: &ValueConfig) -> Result<ModuleEnvelope, CodecError> {
        self.config_codec.encode(config, None)
    }
    fn plan(
        &self,
        target: &TargetIdentity,
        mut snapshot: ValueSnapshot,
        current: &Option<NativeValue>,
        desired: &Option<NativeValue>,
        resources: &[ResourceKey],
    ) -> Plan<ValueSnapshot, ValueStep> {
        let selected = resources.contains(&key(target));
        let steps = if selected && current != desired {
            if !snapshot.known.contains(desired) {
                snapshot.known.push(desired.clone());
            }
            vec![PlannedStep {
                writes: vec![key(target)],
                after: vec![ResourceObservation {
                    key: key(target),
                    value: value_token(desired),
                }],
                instruction: ValueStep {
                    before: current.clone(),
                    after: desired.clone(),
                },
            }]
        } else {
            vec![]
        };
        Plan {
            target: target.clone(),
            claims: if selected {
                vec![claim(target, desired)]
            } else {
                vec![]
            },
            snapshot,
            steps,
            effect: Effect::RestartRequired,
        }
    }
}
impl Module for ValueModule {
    fn snapshot_config_matches(&self, snapshot: &ValueSnapshot, config: &ValueConfig) -> bool {
        snapshot.desired == Some(value_native(config.value))
    }
    type Config = ValueConfig;
    type Observation = Option<NativeValue>;
    type Snapshot = ValueSnapshot;
    type Step = ValueStep;
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            id: FeatureId::new("test_value_setting").unwrap(),
            config_version: SchemaVersion::new(1).unwrap(),
            snapshot_version: SchemaVersion::new(1).unwrap(),
            title: "Test value",
            description: "Distinct single-field test adapter",
            access: ModuleAccess::Managed,
            target_scopes: vec![TargetScope::Session],
            resources: vec![ResourceDeclaration {
                field: ResourceId::new("user_value").unwrap(),
                scope: TargetScope::Session,
            }],
            activation_modes: vec![ActivationMode::Manual],
            effect: Effect::RestartRequired,
        }
    }
    fn config_codec(&self) -> &dyn VersionedCodec<ValueConfig> {
        self.config_codec.as_ref()
    }
    fn snapshot_codec(&self) -> &dyn VersionedCodec<ValueSnapshot> {
        self.snapshot_codec.as_ref()
    }
    fn validate_config(&self, config: &ValueConfig) -> Result<(), ModuleError> {
        if (0..=100).contains(&config.value) {
            Ok(())
        } else {
            Err(ModuleError::new(ErrorKind::InvalidConfig, "value range"))
        }
    }
    fn capability(&self, _: &TargetIdentity) -> Capability {
        Capability::Available
    }
    fn observe(&self, target: &TargetIdentity) -> Result<Self::Observation, ModuleError> {
        Ok(self.backend.observe(target)?.value)
    }
    fn observation_summary(&self, current: &Self::Observation) -> ObservationSummary {
        ObservationSummary {
            summary: format!("value present: {}", current.is_some()),
            effect: Effect::RestartRequired,
        }
    }
    fn observation_states(
        &self,
        target: &TargetIdentity,
        current: &Self::Observation,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        Ok(resources
            .iter()
            .map(|resource| ResourceObservation {
                key: resource.clone(),
                value: value_token(current),
            })
            .filter(|state| state.key == key(target))
            .collect())
    }
    fn prepare(
        &self,
        target: &TargetIdentity,
        config: &ValueConfig,
        current: &Self::Observation,
    ) -> Result<Plan<ValueSnapshot, ValueStep>, ModuleError> {
        let desired = Some(value_native(config.value));
        Ok(self.plan(
            target,
            ValueSnapshot {
                baseline: current.clone(),
                desired: desired.clone(),
                known: vec![],
            },
            current,
            &desired,
            &[key(target)],
        ))
    }
    fn snapshot_claims(
        &self,
        target: &TargetIdentity,
        snapshot: &ValueSnapshot,
    ) -> Result<Vec<ResourceClaim>, ModuleError> {
        if snapshot.known.len() > 256
            || std::iter::once(&snapshot.baseline)
                .chain(std::iter::once(&snapshot.desired))
                .chain(&snapshot.known)
                .any(|value| {
                    value
                        .as_ref()
                        .is_some_and(|native| native.bytes.len() > 4091)
                })
        {
            return Err(ModuleError::new(
                ErrorKind::InvalidConfig,
                "value snapshot bounds",
            ));
        }
        Ok(vec![claim(target, &snapshot.desired)])
    }
    fn baseline_states(
        &self,
        target: &TargetIdentity,
        snapshot: &ValueSnapshot,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        self.observation_states(target, &snapshot.baseline, resources)
    }
    fn same_baseline(&self, left: &ValueSnapshot, right: &ValueSnapshot) -> bool {
        left.baseline == right.baseline
    }
    fn classify(
        &self,
        snapshot: &ValueSnapshot,
        current: &Self::Observation,
        _: &[ResourceKey],
    ) -> Result<StateMatch, ModuleError> {
        Ok(if current == &snapshot.baseline {
            StateMatch::Baseline
        } else if let Some(index) = snapshot.known.iter().position(|value| value == current) {
            StateMatch::WrittenStep(index)
        } else {
            StateMatch::ExternalChange
        })
    }
    fn prepare_update(
        &self,
        target: &TargetIdentity,
        config: &ValueConfig,
        original: &ValueSnapshot,
        current: &Self::Observation,
    ) -> Result<Plan<ValueSnapshot, ValueStep>, ModuleError> {
        let desired = Some(value_native(config.value));
        let mut snapshot = original.clone();
        snapshot.desired = desired.clone();
        Ok(self.plan(target, snapshot, current, &desired, &[key(target)]))
    }
    fn prepare_restore(
        &self,
        target: &TargetIdentity,
        original: &ValueSnapshot,
        current: &Self::Observation,
        resources: &[ResourceKey],
    ) -> Result<Plan<ValueSnapshot, ValueStep>, ModuleError> {
        Ok(self.plan(
            target,
            original.clone(),
            current,
            &original.baseline,
            resources,
        ))
    }
    fn execute_step(
        &self,
        target: &TargetIdentity,
        step: &ValueStep,
    ) -> Result<Effect, ModuleError> {
        self.backend.execute_step(
            target,
            &UserStep::Value {
                before: step.before.clone(),
                after: step.after.clone(),
            },
        )
    }
    fn verify_step(
        &self,
        _: &TargetIdentity,
        step: &ValueStep,
        current: &Self::Observation,
    ) -> Result<Verification, ModuleError> {
        Ok(if current == &step.after {
            Verification::Verified(Effect::RestartRequired)
        } else {
            Verification::Mismatch
        })
    }
}
