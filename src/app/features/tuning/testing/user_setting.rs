//! Test-only two-field session setting. Raw native bytes and absence are exact.
use super::super::platform::{
    catalog::Module,
    codec::{CodecError, EnvelopeKind, JsonCodec, ModuleEnvelope, VersionedCodec},
    contract::*,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    pub enabled: bool,
    pub value: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeValue {
    pub kind: u32,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserObservation {
    value: Option<NativeValue>,
    enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSnapshot {
    baseline: UserObservation,
    desired: UserObservation,
    known: Vec<UserObservation>,
}
pub enum UserStep {
    Value {
        before: Option<NativeValue>,
        after: Option<NativeValue>,
    },
    Enabled {
        before: bool,
        after: bool,
    },
}
struct State {
    current: UserObservation,
    reads: usize,
    writes: usize,
    fail_enabled_step: bool,
}
#[derive(Clone)]
pub struct UserModule {
    state: Arc<Mutex<State>>,
    config_codec: Arc<JsonCodec<UserConfig>>,
    snapshot_codec: Arc<JsonCodec<UserSnapshot>>,
}
fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: FeatureId::new("test_user_setting").unwrap(),
        config_version: SchemaVersion::new(1).unwrap(),
        snapshot_version: SchemaVersion::new(1).unwrap(),
        title: "Test user setting",
        description: "Test-only raw multi-step user setting",
        access: ModuleAccess::Managed,
        target_scopes: vec![TargetScope::Session],
        resources: vec![
            ResourceDeclaration {
                field: ResourceId::new("user_value").unwrap(),
                scope: TargetScope::Session,
            },
            ResourceDeclaration {
                field: ResourceId::new("user_enabled").unwrap(),
                scope: TargetScope::Session,
            },
        ],
        activation_modes: vec![ActivationMode::Manual],
        effect: Effect::RestartRequired,
    }
}
fn value_native(value: i32) -> NativeValue {
    NativeValue {
        kind: 4,
        bytes: value.to_le_bytes().to_vec(),
    }
}
fn value_token(value: &Option<NativeValue>) -> DesiredStateToken {
    let mut bytes = vec![0];
    if let Some(value) = value {
        bytes[0] = 1;
        bytes.extend_from_slice(&value.kind.to_le_bytes());
        bytes.extend_from_slice(&value.bytes);
    }
    DesiredStateToken::new(
        ResourceId::new("native_raw_value").unwrap(),
        SchemaVersion::new(1).unwrap(),
        bytes,
    )
    .unwrap()
}
fn enabled_token(value: bool) -> DesiredStateToken {
    DesiredStateToken::new(
        ResourceId::new("boolean").unwrap(),
        SchemaVersion::new(1).unwrap(),
        vec![u8::from(value)],
    )
    .unwrap()
}
fn key(target: &TargetIdentity, field: &str) -> ResourceKey {
    ResourceKey {
        target: target.clone(),
        field: ResourceId::new(field).unwrap(),
    }
}
fn claims(target: &TargetIdentity, current: &UserObservation) -> Vec<ResourceClaim> {
    vec![
        ResourceClaim {
            key: key(target, "user_value"),
            desired: value_token(&current.value),
        },
        ResourceClaim {
            key: key(target, "user_enabled"),
            desired: enabled_token(current.enabled),
        },
    ]
}
fn equal_fields(
    left: &UserObservation,
    right: &UserObservation,
    resources: &[ResourceKey],
) -> bool {
    resources
        .iter()
        .all(|resource| match resource.field.as_str() {
            "user_value" => left.value == right.value,
            "user_enabled" => left.enabled == right.enabled,
            _ => false,
        })
}
fn remember_written_fields(
    snapshot: &mut UserSnapshot,
    merged: &UserObservation,
    resources: &[ResourceKey],
) {
    let writes_value = resources
        .iter()
        .any(|resource| resource.field.as_str() == "user_value");
    let writes_enabled = resources
        .iter()
        .any(|resource| resource.field.as_str() == "user_enabled");
    // A partial restore must never promote unrelated external observations to
    // authored history. Untouched values need pre-existing baseline/write evidence.
    let known_value = writes_value
        || snapshot.baseline.value == merged.value
        || snapshot
            .known
            .iter()
            .any(|known| known.value == merged.value);
    let known_enabled = writes_enabled
        || snapshot.baseline.enabled == merged.enabled
        || snapshot
            .known
            .iter()
            .any(|known| known.enabled == merged.enabled);
    if known_value && known_enabled && !snapshot.known.contains(merged) {
        snapshot.known.push(merged.clone());
    }
}
impl UserModule {
    pub fn new(value: Option<i32>, enabled: bool) -> Self {
        Self::new_native(value.map(value_native), enabled)
    }
    pub fn new_native(value: Option<NativeValue>, enabled: bool) -> Self {
        let descriptor = descriptor();
        Self {
            state: Arc::new(Mutex::new(State {
                current: UserObservation { value, enabled },
                reads: 0,
                writes: 0,
                fail_enabled_step: false,
            })),
            config_codec: Arc::new(JsonCodec::new(
                descriptor.id.clone(),
                EnvelopeKind::Config,
                descriptor.config_version,
            )),
            snapshot_codec: Arc::new(JsonCodec::new(
                descriptor.id,
                EnvelopeKind::Snapshot,
                descriptor.snapshot_version,
            )),
        }
    }
    pub fn config_envelope(&self, config: &UserConfig) -> Result<ModuleEnvelope, CodecError> {
        self.config_codec.encode(config, None)
    }
    pub fn reads(&self) -> usize {
        self.state.lock().unwrap().reads
    }
    pub fn writes(&self) -> usize {
        self.state.lock().unwrap().writes
    }
    pub fn native_value(&self) -> Option<NativeValue> {
        self.state.lock().unwrap().current.value.clone()
    }
    pub fn value(&self) -> Option<i32> {
        self.native_value()
            .and_then(|value| value.bytes.try_into().ok().map(i32::from_le_bytes))
    }
    pub fn enabled(&self) -> bool {
        self.state.lock().unwrap().current.enabled
    }
    pub fn external_enabled(&self, enabled: bool) {
        self.state.lock().unwrap().current.enabled = enabled;
    }
    pub fn external_value(&self, value: Option<NativeValue>) {
        self.state.lock().unwrap().current.value = value;
    }
    pub fn fail_next_enabled_step(&self) {
        self.state.lock().unwrap().fail_enabled_step = true;
    }
    fn plan(
        &self,
        target: &TargetIdentity,
        mut snapshot: UserSnapshot,
        current: &UserObservation,
        desired: &UserObservation,
        resources: &[ResourceKey],
    ) -> Plan<UserSnapshot, UserStep> {
        let mut merged = current.clone();
        let mut steps = Vec::new();
        if resources.contains(&key(target, "user_value")) && current.value != desired.value {
            steps.push(PlannedStep {
                writes: vec![key(target, "user_value")],
                instruction: UserStep::Value {
                    before: current.value.clone(),
                    after: desired.value.clone(),
                },
            });
            merged.value = desired.value.clone();
            remember_written_fields(&mut snapshot, &merged, resources);
        }
        if resources.contains(&key(target, "user_enabled")) && current.enabled != desired.enabled {
            steps.push(PlannedStep {
                writes: vec![key(target, "user_enabled")],
                instruction: UserStep::Enabled {
                    before: current.enabled,
                    after: desired.enabled,
                },
            });
            merged.enabled = desired.enabled;
            remember_written_fields(&mut snapshot, &merged, resources);
        }
        Plan {
            target: target.clone(),
            claims: claims(target, desired)
                .into_iter()
                .filter(|claim| resources.contains(&claim.key))
                .collect(),
            snapshot,
            steps,
            effect: Effect::RestartRequired,
        }
    }
}
impl Module for UserModule {
    type Config = UserConfig;
    type Observation = UserObservation;
    type Snapshot = UserSnapshot;
    type Step = UserStep;
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }
    fn config_codec(&self) -> &dyn VersionedCodec<UserConfig> {
        self.config_codec.as_ref()
    }
    fn snapshot_codec(&self) -> &dyn VersionedCodec<UserSnapshot> {
        self.snapshot_codec.as_ref()
    }
    fn validate_config(&self, config: &UserConfig) -> Result<(), ModuleError> {
        if (0..=100).contains(&config.value) {
            Ok(())
        } else {
            Err(ModuleError::new(
                ErrorKind::InvalidConfig,
                "user value range",
            ))
        }
    }
    fn capability(&self, _: &TargetIdentity) -> Capability {
        Capability::Available
    }
    fn observe(&self, _: &TargetIdentity) -> Result<UserObservation, ModuleError> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        Ok(state.current.clone())
    }
    fn observation_summary(&self, observation: &UserObservation) -> ObservationSummary {
        ObservationSummary {
            summary: format!(
                "value present: {}; enabled: {}",
                observation.value.is_some(),
                observation.enabled
            ),
            effect: Effect::RestartRequired,
        }
    }
    fn observation_states(
        &self,
        target: &TargetIdentity,
        observation: &UserObservation,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        Ok(claims(target, observation)
            .into_iter()
            .filter(|claim| resources.contains(&claim.key))
            .map(|claim| ResourceObservation {
                key: claim.key,
                value: claim.desired,
            })
            .collect())
    }
    fn prepare(
        &self,
        target: &TargetIdentity,
        config: &UserConfig,
        current: &UserObservation,
    ) -> Result<Plan<UserSnapshot, UserStep>, ModuleError> {
        let desired = UserObservation {
            value: Some(value_native(config.value)),
            enabled: config.enabled,
        };
        let resources = claims(target, &desired)
            .into_iter()
            .map(|claim| claim.key)
            .collect::<Vec<_>>();
        Ok(self.plan(
            target,
            UserSnapshot {
                baseline: current.clone(),
                desired: desired.clone(),
                known: vec![],
            },
            current,
            &desired,
            &resources,
        ))
    }
    fn snapshot_claims(
        &self,
        target: &TargetIdentity,
        snapshot: &UserSnapshot,
    ) -> Result<Vec<ResourceClaim>, ModuleError> {
        if snapshot.known.len() > 256
            || std::iter::once(&snapshot.baseline)
                .chain(std::iter::once(&snapshot.desired))
                .chain(&snapshot.known)
                .any(|state| {
                    state
                        .value
                        .as_ref()
                        .is_some_and(|value| value.bytes.len() > 4091)
                })
        {
            return Err(ModuleError::new(
                ErrorKind::InvalidConfig,
                "raw snapshot limits",
            ));
        }
        Ok(claims(target, &snapshot.desired))
    }
    fn same_baseline(&self, left: &UserSnapshot, right: &UserSnapshot) -> bool {
        left.baseline == right.baseline
    }
    fn classify(
        &self,
        snapshot: &UserSnapshot,
        current: &UserObservation,
        resources: &[ResourceKey],
    ) -> Result<StateMatch, ModuleError> {
        Ok(if equal_fields(&snapshot.baseline, current, resources) {
            StateMatch::Baseline
        } else if let Some(index) = snapshot
            .known
            .iter()
            .position(|known| equal_fields(known, current, resources))
        {
            StateMatch::WrittenStep(index)
        } else {
            StateMatch::ExternalChange
        })
    }
    fn prepare_update(
        &self,
        target: &TargetIdentity,
        config: &UserConfig,
        original: &UserSnapshot,
        current: &UserObservation,
    ) -> Result<Plan<UserSnapshot, UserStep>, ModuleError> {
        let desired = UserObservation {
            value: Some(value_native(config.value)),
            enabled: config.enabled,
        };
        let mut snapshot = original.clone();
        snapshot.desired = desired.clone();
        let resources = claims(target, &desired)
            .into_iter()
            .map(|claim| claim.key)
            .collect::<Vec<_>>();
        Ok(self.plan(target, snapshot, current, &desired, &resources))
    }
    fn prepare_restore(
        &self,
        target: &TargetIdentity,
        original: &UserSnapshot,
        current: &UserObservation,
        released: &[ResourceKey],
    ) -> Result<Plan<UserSnapshot, UserStep>, ModuleError> {
        Ok(self.plan(
            target,
            original.clone(),
            current,
            &original.baseline,
            released,
        ))
    }
    fn execute_step(&self, _: &TargetIdentity, step: &UserStep) -> Result<Effect, ModuleError> {
        let mut state = self.state.lock().unwrap();
        match step {
            UserStep::Value { before, after } => {
                if state.current.value != *before {
                    return Err(ModuleError::new(
                        ErrorKind::ExternalChange,
                        "user conditional value write",
                    ));
                }
                state.current.value = after.clone();
            }
            UserStep::Enabled { before, after } => {
                if state.current.enabled != *before {
                    return Err(ModuleError::new(
                        ErrorKind::ExternalChange,
                        "user conditional flag write",
                    ));
                }
                if state.fail_enabled_step {
                    state.fail_enabled_step = false;
                    return Err(
                        ModuleError::new(ErrorKind::Write, "user simulated second step")
                            .with_native(NativeError {
                                domain: ResourceId::new("win32").unwrap(),
                                code: 5,
                            })
                            .with_detail("Simulated access denied")
                            .unwrap(),
                    );
                }
                state.current.enabled = *after;
            }
        }
        state.writes += 1;
        Ok(Effect::RestartRequired)
    }
    fn verify_step(
        &self,
        _: &TargetIdentity,
        step: &UserStep,
        current: &UserObservation,
    ) -> Result<Verification, ModuleError> {
        let matches = match step {
            UserStep::Value { after, .. } => current.value == *after,
            UserStep::Enabled { after, .. } => current.enabled == *after,
        };
        Ok(if matches {
            Verification::Verified(Effect::RestartRequired)
        } else {
            Verification::Mismatch
        })
    }
}
