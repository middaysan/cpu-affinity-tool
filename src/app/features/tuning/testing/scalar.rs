//! Deliberately test-only machine scalar adapter, with deterministic native state.
use super::super::platform::{
    catalog::Module,
    codec::{CodecError, EnvelopeKind, JsonCodec, ModuleEnvelope, VersionedCodec},
    contract::*,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarConfig {
    pub value: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarSnapshot {
    baseline: i32,
    desired: i32,
    known: Vec<i32>,
}
pub struct ScalarStep {
    before: i32,
    after: i32,
}
struct State {
    value: i32,
    capability: Capability,
    reads: usize,
    writes: usize,
    invalid_plan: bool,
    invalid_snapshot: bool,
    invalid_step: bool,
    plan_via: Vec<i32>,
    native_effect: Option<Effect>,
}
#[derive(Default)]
struct Hooks {
    observe: Option<Arc<dyn Fn() + Send + Sync>>,
    execute: Option<Arc<dyn Fn() + Send + Sync>>,
}
#[derive(Clone)]
pub struct ScalarModule {
    state: Arc<Mutex<State>>,
    hooks: Arc<Mutex<Hooks>>,
    descriptor: ModuleDescriptor,
    config_codec: Arc<JsonCodec<ScalarConfig>>,
    snapshot_codec: Arc<JsonCodec<ScalarSnapshot>>,
}

pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: FeatureId::new("test_scalar").unwrap(),
        config_version: SchemaVersion::new(1).unwrap(),
        snapshot_version: SchemaVersion::new(1).unwrap(),
        title: "Test scalar",
        description: "Test-only scalar adapter",
        access: ModuleAccess::Managed,
        target_scopes: vec![TargetScope::Machine],
        resources: vec![ResourceDeclaration {
            field: ResourceId::new("scalar_setting").unwrap(),
            scope: TargetScope::Machine,
        }],
        activation_modes: vec![ActivationMode::Manual, ActivationMode::Session],
        effect: Effect::Immediate,
    }
}
pub fn invalid_descriptors() -> Vec<ModuleDescriptor> {
    let mut result = Vec::new();
    let mut d = descriptor();
    d.title = " ";
    result.push(d);
    let mut d = descriptor();
    d.target_scopes.clear();
    result.push(d);
    let mut d = descriptor();
    d.target_scopes.push(TargetScope::Machine);
    result.push(d);
    let mut d = descriptor();
    d.resources.clear();
    result.push(d);
    let mut d = descriptor();
    d.resources.push(d.resources[0].clone());
    result.push(d);
    let mut d = descriptor();
    d.activation_modes.clear();
    result.push(d);
    let mut d = descriptor();
    d.activation_modes.push(ActivationMode::Manual);
    result.push(d);
    let mut d = descriptor();
    d.effect = Effect::RestartRequired;
    result.push(d);
    let mut d = descriptor();
    d.access = ModuleAccess::ReadOnly;
    result.push(d);
    result
}
fn token(value: i32) -> DesiredStateToken {
    DesiredStateToken::new(
        ResourceId::new("signed_i32_le").unwrap(),
        SchemaVersion::new(1).unwrap(),
        value.to_le_bytes().to_vec(),
    )
    .unwrap()
}
impl ScalarModule {
    fn key(&self, target: &TargetIdentity) -> ResourceKey {
        let declaration = &self.descriptor.resources[0];
        ResourceKey {
            target: target.resource_target(declaration.scope).unwrap(),
            field: declaration.field.clone(),
        }
    }
    fn claim(&self, target: &TargetIdentity, value: i32) -> ResourceClaim {
        ResourceClaim {
            key: self.key(target),
            desired: token(value),
        }
    }
    pub fn new(value: i32) -> Self {
        let mut result = Self::with_descriptor(descriptor());
        result.state = Arc::new(Mutex::new(State {
            value,
            capability: Capability::Available,
            reads: 0,
            writes: 0,
            invalid_plan: false,
            invalid_snapshot: false,
            invalid_step: false,
            plan_via: vec![],
            native_effect: None,
        }));
        result
    }
    pub fn with_descriptor(descriptor: ModuleDescriptor) -> Self {
        Self {
            hooks: Arc::new(Mutex::new(Hooks::default())),
            config_codec: Arc::new(JsonCodec::new(
                descriptor.id.clone(),
                EnvelopeKind::Config,
                descriptor.config_version,
            )),
            snapshot_codec: Arc::new(JsonCodec::new(
                descriptor.id.clone(),
                EnvelopeKind::Snapshot,
                descriptor.snapshot_version,
            )),
            descriptor,
            state: Arc::new(Mutex::new(State {
                value: 7,
                capability: Capability::Available,
                reads: 0,
                writes: 0,
                invalid_plan: false,
                invalid_snapshot: false,
                invalid_step: false,
                plan_via: vec![],
                native_effect: None,
            })),
        }
    }
    pub fn config_envelope(&self, config: &ScalarConfig) -> Result<ModuleEnvelope, CodecError> {
        self.config_codec.encode(config, None)
    }
    pub fn reads(&self) -> usize {
        self.state.lock().unwrap().reads
    }
    pub fn set_observe_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.hooks.lock().unwrap().observe = Some(hook);
    }
    pub fn set_execute_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.hooks.lock().unwrap().execute = Some(hook);
    }
    pub fn clear_observe_hook(&self) {
        self.hooks.lock().unwrap().observe = None;
    }
    pub fn set_plan_via(&self, values: Vec<i32>) {
        self.state.lock().unwrap().plan_via = values;
    }
    pub fn return_next_native_effect(&self, effect: Effect) {
        self.state.lock().unwrap().native_effect = Some(effect);
    }
    pub fn writes(&self) -> usize {
        self.state.lock().unwrap().writes
    }
    pub fn value(&self) -> i32 {
        self.state.lock().unwrap().value
    }
    pub fn external_change(&self, value: i32) {
        self.state.lock().unwrap().value = value;
    }
    pub fn set_capability(&self, capability: Capability) {
        self.state.lock().unwrap().capability = capability;
    }
    pub fn set_invalid_plan(&self, invalid: bool) {
        self.state.lock().unwrap().invalid_plan = invalid;
    }
    pub fn set_invalid_snapshot(&self, invalid: bool) {
        self.state.lock().unwrap().invalid_snapshot = invalid;
    }
    pub fn set_invalid_step(&self, invalid: bool) {
        self.state.lock().unwrap().invalid_step = invalid;
    }
    pub fn with_config_codec(
        mut self,
        feature: FeatureId,
        kind: EnvelopeKind,
        version: SchemaVersion,
    ) -> Self {
        self.config_codec = Arc::new(JsonCodec::new(feature, kind, version));
        self
    }
    fn plan(
        &self,
        target: &TargetIdentity,
        mut snapshot: ScalarSnapshot,
        before: i32,
        after: i32,
    ) -> Plan<ScalarSnapshot, ScalarStep> {
        let mut step_key = self.key(target);
        if self.state.lock().unwrap().invalid_step {
            step_key.field = ResourceId::new("foreign_setting").unwrap();
        }
        let mut values = if snapshot.desired == after {
            self.state.lock().unwrap().plan_via.clone()
        } else {
            vec![]
        };
        values.push(after);
        let mut previous = before;
        let steps = values
            .into_iter()
            .filter_map(|value| {
                if !snapshot.known.contains(&value) {
                    snapshot.known.push(value);
                }
                if previous == value {
                    return None;
                }
                let step = PlannedStep {
                    writes: vec![step_key.clone()],
                    after: vec![ResourceObservation {
                        key: self.key(target),
                        value: token(value),
                    }],
                    instruction: ScalarStep {
                        before: previous,
                        after: value,
                    },
                };
                previous = value;
                Some(step)
            })
            .collect();
        let claims = if self.state.lock().unwrap().invalid_plan {
            vec![]
        } else {
            vec![self.claim(target, after)]
        };
        Plan {
            target: target.clone(),
            claims,
            snapshot,
            steps,
            effect: Effect::Immediate,
        }
    }
}
impl Module for ScalarModule {
    type Config = ScalarConfig;
    type Observation = i32;
    type Snapshot = ScalarSnapshot;
    type Step = ScalarStep;
    fn descriptor(&self) -> ModuleDescriptor {
        self.descriptor.clone()
    }
    fn config_codec(&self) -> &dyn VersionedCodec<Self::Config> {
        self.config_codec.as_ref()
    }
    fn snapshot_codec(&self) -> &dyn VersionedCodec<Self::Snapshot> {
        self.snapshot_codec.as_ref()
    }
    fn validate_config(&self, config: &Self::Config) -> Result<(), ModuleError> {
        if (0..=100).contains(&config.value) {
            Ok(())
        } else {
            Err(ModuleError::new(ErrorKind::InvalidConfig, "scalar range"))
        }
    }
    fn capability(&self, _: &TargetIdentity) -> Capability {
        self.state.lock().unwrap().capability.clone()
    }
    fn observe(&self, _: &TargetIdentity) -> Result<i32, ModuleError> {
        let hook = self.hooks.lock().unwrap().observe.clone();
        if let Some(hook) = hook {
            hook();
        }
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        Ok(state.value)
    }
    fn observation_summary(&self, observation: &i32) -> ObservationSummary {
        ObservationSummary {
            summary: observation.to_string(),
            effect: Effect::Immediate,
        }
    }
    fn observation_states(
        &self,
        target: &TargetIdentity,
        observation: &i32,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        Ok(resources
            .iter()
            .map(|resource| ResourceObservation {
                key: resource.clone(),
                value: token(*observation),
            })
            .filter(|state| state.key == self.key(target))
            .collect())
    }
    fn prepare(
        &self,
        target: &TargetIdentity,
        config: &ScalarConfig,
        current: &i32,
    ) -> Result<Plan<ScalarSnapshot, ScalarStep>, ModuleError> {
        Ok(self.plan(
            target,
            ScalarSnapshot {
                baseline: *current,
                desired: config.value,
                known: vec![config.value],
            },
            *current,
            config.value,
        ))
    }
    fn snapshot_claims(
        &self,
        target: &TargetIdentity,
        snapshot: &ScalarSnapshot,
    ) -> Result<Vec<ResourceClaim>, ModuleError> {
        self.validate_config(&ScalarConfig {
            value: snapshot.desired,
        })?;
        if snapshot.known.len() > 256 {
            return Err(ModuleError::new(
                ErrorKind::InvalidConfig,
                "snapshot history",
            ));
        }
        Ok(if self.state.lock().unwrap().invalid_snapshot {
            vec![]
        } else {
            vec![self.claim(target, snapshot.desired)]
        })
    }
    fn same_baseline(&self, left: &ScalarSnapshot, right: &ScalarSnapshot) -> bool {
        left.baseline == right.baseline
    }
    fn baseline_states(
        &self,
        target: &TargetIdentity,
        snapshot: &ScalarSnapshot,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        Ok(resources
            .iter()
            .map(|resource| ResourceObservation {
                key: resource.clone(),
                value: token(snapshot.baseline),
            })
            .filter(|state| state.key == self.key(target))
            .collect())
    }
    fn classify(
        &self,
        snapshot: &ScalarSnapshot,
        current: &i32,
        _: &[ResourceKey],
    ) -> Result<StateMatch, ModuleError> {
        Ok(if *current == snapshot.baseline {
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
        config: &ScalarConfig,
        original: &ScalarSnapshot,
        current: &i32,
    ) -> Result<Plan<ScalarSnapshot, ScalarStep>, ModuleError> {
        let mut snapshot = original.clone();
        snapshot.desired = config.value;
        if !snapshot.known.contains(&config.value) {
            snapshot.known.push(config.value);
        }
        Ok(self.plan(target, snapshot, *current, config.value))
    }
    fn prepare_restore(
        &self,
        target: &TargetIdentity,
        original: &ScalarSnapshot,
        current: &i32,
        released: &[ResourceKey],
    ) -> Result<Plan<ScalarSnapshot, ScalarStep>, ModuleError> {
        if released.is_empty() {
            return Ok(Plan {
                target: target.clone(),
                claims: vec![],
                snapshot: original.clone(),
                steps: vec![],
                effect: Effect::Immediate,
            });
        }
        Ok(self.plan(target, original.clone(), *current, original.baseline))
    }
    fn execute_step(&self, _: &TargetIdentity, step: &ScalarStep) -> Result<Effect, ModuleError> {
        let hook = self.hooks.lock().unwrap().execute.clone();
        if let Some(hook) = hook {
            hook();
        }
        let mut state = self.state.lock().unwrap();
        if state.value != step.before {
            return Err(ModuleError::new(
                ErrorKind::ExternalChange,
                "scalar conditional write",
            ));
        }
        state.value = step.after;
        state.writes += 1;
        Ok(state.native_effect.take().unwrap_or(Effect::Immediate))
    }
    fn verify_step(
        &self,
        _: &TargetIdentity,
        step: &ScalarStep,
        current: &i32,
    ) -> Result<Verification, ModuleError> {
        Ok(if *current == step.after {
            Verification::Verified(Effect::Immediate)
        } else {
            Verification::Mismatch
        })
    }
}
