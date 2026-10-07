//! Static heterogeneous composition with typed module state behind opaque operations.
use super::codec::{CodecError, EnvelopeKind, ModuleEnvelope, VersionedCodec};
use super::contract::*;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_RESOURCES: usize = 64;
const MAX_STEPS: usize = 256;

/// Modules own concrete values, native step semantics, and codecs. Preparation
/// only reads. Native writes must revalidate target provenance on the write handle.
pub trait Module: Send + Sync + 'static {
    type Config: Clone + Eq + Send + Sync + 'static;
    type Observation: Send + Sync + 'static;
    type Snapshot: Clone + Eq + Send + Sync + 'static;
    type Step: Send + Sync + 'static;

    fn descriptor(&self) -> ModuleDescriptor;
    fn config_codec(&self) -> &dyn VersionedCodec<Self::Config>;
    fn snapshot_codec(&self) -> &dyn VersionedCodec<Self::Snapshot>;
    fn validate_config(&self, config: &Self::Config) -> Result<(), ModuleError>;
    /// Pure recovery validation; never observe native state or recapture an
    /// original. Config and snapshot must describe the same typed request.
    fn snapshot_config_matches(&self, snapshot: &Self::Snapshot, config: &Self::Config) -> bool;
    fn capability(&self, target: &TargetIdentity) -> Capability;
    fn observe(&self, target: &TargetIdentity) -> Result<Self::Observation, ModuleError>;
    fn observation_summary(&self, observation: &Self::Observation) -> ObservationSummary;
    fn observation_states(
        &self,
        target: &TargetIdentity,
        observation: &Self::Observation,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError>;
    fn prepare(
        &self,
        target: &TargetIdentity,
        config: &Self::Config,
        current: &Self::Observation,
    ) -> Result<Plan<Self::Snapshot, Self::Step>, ModuleError>;
    fn snapshot_claims(
        &self,
        target: &TargetIdentity,
        snapshot: &Self::Snapshot,
    ) -> Result<Vec<ResourceClaim>, ModuleError>;
    fn baseline_states(
        &self,
        target: &TargetIdentity,
        snapshot: &Self::Snapshot,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError>;
    fn same_baseline(&self, left: &Self::Snapshot, right: &Self::Snapshot) -> bool;
    fn classify(
        &self,
        snapshot: &Self::Snapshot,
        current: &Self::Observation,
        resources: &[ResourceKey],
    ) -> Result<StateMatch, ModuleError>;
    /// Retains the first original baseline AND earlier known app-written states.
    fn prepare_update(
        &self,
        target: &TargetIdentity,
        config: &Self::Config,
        original: &Self::Snapshot,
        current: &Self::Observation,
    ) -> Result<Plan<Self::Snapshot, Self::Step>, ModuleError>;
    /// Merges current untouched fields and restores only the released resources.
    fn prepare_restore(
        &self,
        target: &TargetIdentity,
        original: &Self::Snapshot,
        current: &Self::Observation,
        released: &[ResourceKey],
    ) -> Result<Plan<Self::Snapshot, Self::Step>, ModuleError>;
    fn execute_step(
        &self,
        target: &TargetIdentity,
        step: &Self::Step,
    ) -> Result<Effect, ModuleError>;
    fn verify_step(
        &self,
        target: &TargetIdentity,
        step: &Self::Step,
        current: &Self::Observation,
    ) -> Result<Verification, ModuleError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    DuplicateFeature(FeatureId),
    UnknownFeature(FeatureId),
    InvalidDescriptor(&'static str),
    WrongScope,
    WrongActivation,
    ReadOnly,
    InvalidPlan(&'static str),
    InvalidResourceSubset,
    StepOutOfBounds,
    Codec(CodecError),
    Module(ModuleError),
}
impl From<CodecError> for CatalogError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}
impl From<ModuleError> for CatalogError {
    fn from(value: ModuleError) -> Self {
        Self::Module(value)
    }
}

fn distinct<T: Ord>(values: impl IntoIterator<Item = T>) -> bool {
    let mut seen = BTreeSet::new();
    values.into_iter().all(|value| seen.insert(value))
}
fn check_descriptor<M: Module>(
    module: &M,
    descriptor: &ModuleDescriptor,
) -> Result<(), CatalogError> {
    let bad = |reason| Err(CatalogError::InvalidDescriptor(reason));
    if descriptor.title.trim().is_empty() || descriptor.description.trim().is_empty() {
        return bad("empty presentation");
    }
    if descriptor.target_scopes.is_empty() || !distinct(descriptor.target_scopes.iter()) {
        return bad("invalid target scopes");
    }
    if descriptor.resources.len() > MAX_RESOURCES || !distinct(descriptor.resources.iter()) {
        return bad("invalid resources");
    }
    if descriptor.resources.iter().any(|resource| {
        !descriptor
            .target_scopes
            .iter()
            .any(|scope| scope.can_project_to(resource.scope))
    }) {
        return bad("unreachable resource scope");
    }
    if !distinct(descriptor.activation_modes.iter()) {
        return bad("duplicate activation modes");
    }
    if descriptor.access == ModuleAccess::Managed
        && (descriptor.resources.is_empty() || descriptor.activation_modes.is_empty())
    {
        return bad("managed module lacks resource or activation");
    }
    if descriptor.access == ModuleAccess::ReadOnly
        && (!descriptor.resources.is_empty() || !descriptor.activation_modes.is_empty())
    {
        return bad("read-only module declares mutation");
    }
    if descriptor.effect == Effect::RestartRequired
        && descriptor
            .activation_modes
            .iter()
            .any(|mode| *mode != ActivationMode::Manual)
    {
        return bad("restart setting cannot be session managed");
    }
    if module.config_codec().version() != descriptor.config_version
        || module.snapshot_codec().version() != descriptor.snapshot_version
    {
        return bad("codec version mismatch");
    }
    if module.config_codec().feature() != &descriptor.id
        || module.snapshot_codec().feature() != &descriptor.id
        || module.config_codec().kind() != EnvelopeKind::Config
        || module.snapshot_codec().kind() != EnvelopeKind::Snapshot
    {
        return bad("codec identity mismatch");
    }
    Ok(())
}
fn check_target(
    descriptor: &ModuleDescriptor,
    target: &TargetIdentity,
) -> Result<(), CatalogError> {
    if descriptor.target_scopes.contains(&target.scope()) {
        Ok(())
    } else {
        Err(CatalogError::WrongScope)
    }
}
fn check_capability<M: Module>(module: &M, target: &TargetIdentity) -> Result<(), CatalogError> {
    match module.capability(target) {
        Capability::Available => Ok(()),
        Capability::Unsupported(reason) => {
            Err(ModuleError::new(ErrorKind::Unsupported, "capability")
                .with_detail(reason)
                .map_err(|_| CatalogError::InvalidPlan("unbounded capability reason"))?
                .into())
        }
        Capability::Denied(reason) => Err(ModuleError::new(ErrorKind::Denied, "capability")
            .with_detail(reason)
            .map_err(|_| CatalogError::InvalidPlan("unbounded capability reason"))?
            .into()),
    }
}
fn check_claims(
    descriptor: &ModuleDescriptor,
    target: &TargetIdentity,
    claims: &[ResourceClaim],
) -> Result<(), CatalogError> {
    if claims.is_empty()
        || claims.len() > MAX_RESOURCES
        || !distinct(claims.iter().map(|claim| &claim.key))
    {
        return Err(CatalogError::InvalidPlan("invalid claims"));
    }
    for claim in claims {
        if !descriptor.resources.iter().any(|declared| {
            declared.field == claim.key.field
                && target.resource_target(declared.scope).as_ref() == Some(&claim.key.target)
        }) {
            return Err(CatalogError::InvalidPlan("undeclared or foreign resource"));
        }
    }
    Ok(())
}
fn keys(claims: &[ResourceClaim]) -> Vec<ResourceKey> {
    claims.iter().map(|claim| claim.key.clone()).collect()
}
fn check_subset(
    available: &[ResourceClaim],
    requested: &[ResourceKey],
) -> Result<(), CatalogError> {
    if !distinct(requested.iter())
        || requested
            .iter()
            .any(|key| !available.iter().any(|claim| claim.key == *key))
    {
        return Err(CatalogError::InvalidResourceSubset);
    }
    Ok(())
}
fn check_plan<S, Step>(
    descriptor: &ModuleDescriptor,
    target: &TargetIdentity,
    plan: &Plan<S, Step>,
) -> Result<(), CatalogError> {
    if &plan.target != target || plan.effect != descriptor.effect || plan.steps.len() > MAX_STEPS {
        return Err(CatalogError::InvalidPlan(
            "target, effect or step count mismatch",
        ));
    }
    if !plan.claims.is_empty() {
        check_claims(descriptor, target, &plan.claims)?;
    }
    for step in &plan.steps {
        if step.writes.is_empty()
            || !distinct(step.writes.iter())
            || step
                .writes
                .iter()
                .any(|key| !plan.claims.iter().any(|claim| claim.key == *key))
        {
            return Err(CatalogError::InvalidPlan("step writes undeclared resource"));
        }
        check_observations(&step.after, &step.writes)?;
        if step.after.iter().any(|state| {
            !plan.claims.iter().any(|claim| {
                claim.key == state.key
                    && claim.desired.semantics == state.value.semantics
                    && claim.desired.version == state.value.version
            })
        }) {
            return Err(CatalogError::InvalidPlan(
                "step after-state semantics mismatch",
            ));
        }
    }
    Ok(())
}

fn check_snapshot<M: Module>(
    module: &M,
    descriptor: &ModuleDescriptor,
    plan: &Plan<M::Snapshot, M::Step>,
    applying: bool,
) -> Result<(), CatalogError> {
    let original = module.snapshot_claims(&plan.target, &plan.snapshot)?;
    check_claims(descriptor, &plan.target, &original)?;
    if applying {
        if original.len() != plan.claims.len()
            || original.iter().any(|claim| !plan.claims.contains(claim))
        {
            return Err(CatalogError::InvalidPlan(
                "snapshot claims differ from apply claims",
            ));
        }
    } else {
        check_subset(&original, &keys(&plan.claims))?;
    }
    // Prove the prepared recovery artifact fits its codec before any executor
    // is exposed. This is a codec boundary check, not JSON runtime state.
    let encoded = module
        .snapshot_codec()
        .encode(&plan.snapshot, Some(&plan.target))?;
    encoded.to_bytes()?;
    if module.snapshot_codec().decode(&encoded)? != plan.snapshot {
        return Err(CatalogError::InvalidPlan(
            "snapshot codec changed typed state",
        ));
    }
    Ok(())
}

trait ErasedModule: Send + Sync {
    fn recover_source(
        &self,
        snapshot: &ModuleEnvelope,
        config: Option<&ModuleEnvelope>,
    ) -> Result<PreparedOperation, CatalogError>;
    fn descriptor(&self) -> &ModuleDescriptor;
    fn validate_config(&self, envelope: &ModuleEnvelope) -> Result<(), CatalogError>;
    fn observe(&self, target: &TargetIdentity) -> Result<ObservationSummary, CatalogError>;
    fn observe_resources(
        &self,
        target: &TargetIdentity,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, CatalogError>;
    fn prepare(
        &self,
        envelope: &ModuleEnvelope,
        target: TargetIdentity,
        mode: ActivationMode,
    ) -> Result<PreparedOperation, CatalogError>;
    fn prepare_restore(
        &self,
        envelope: &ModuleEnvelope,
        target: TargetIdentity,
        released: Option<&[ResourceKey]>,
    ) -> Result<PreparedOperation, CatalogError>;
}
struct ModuleAdapter<M: Module> {
    module: Arc<M>,
    descriptor: ModuleDescriptor,
}
impl<M: Module> ModuleAdapter<M> {
    fn bind(
        &self,
        config: Option<M::Config>,
        plan: Plan<M::Snapshot, M::Step>,
    ) -> Result<PreparedOperation, CatalogError> {
        check_plan(&self.descriptor, &plan.target, &plan)?;
        check_snapshot(
            self.module.as_ref(),
            &self.descriptor,
            &plan,
            config.is_some(),
        )?;
        Ok(PreparedOperation {
            inner: Box::new(TypedOperation {
                module: Arc::clone(&self.module),
                descriptor: self.descriptor.clone(),
                cleanup: config.is_none(),
                config,
                plan,
            }),
        })
    }
    fn decode_config(&self, envelope: &ModuleEnvelope) -> Result<M::Config, CatalogError> {
        let config = self.module.config_codec().decode(envelope)?;
        self.module.validate_config(&config)?;
        Ok(config)
    }
}
impl<M: Module> ErasedModule for ModuleAdapter<M> {
    fn recover_source(
        &self,
        envelope: &ModuleEnvelope,
        config: Option<&ModuleEnvelope>,
    ) -> Result<PreparedOperation, CatalogError> {
        let target = envelope.target.clone().ok_or(CodecError::WrongTarget)?;
        check_target(&self.descriptor, &target)?;
        if self.descriptor.access != ModuleAccess::Managed {
            return Err(CatalogError::ReadOnly);
        }
        let snapshot = self.module.snapshot_codec().decode(envelope)?;
        let claims = self.module.snapshot_claims(&target, &snapshot)?;
        let config = config
            .map(|envelope| self.decode_config(envelope))
            .transpose()?;
        if config
            .as_ref()
            .is_some_and(|config| !self.module.snapshot_config_matches(&snapshot, config))
        {
            return Err(CatalogError::InvalidPlan(
                "snapshot request differs from configuration",
            ));
        }
        self.bind(
            config,
            Plan {
                target,
                claims,
                snapshot,
                steps: vec![],
                effect: self.descriptor.effect,
            },
        )
    }
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn validate_config(&self, envelope: &ModuleEnvelope) -> Result<(), CatalogError> {
        self.decode_config(envelope).map(|_| ())
    }
    fn observe(&self, target: &TargetIdentity) -> Result<ObservationSummary, CatalogError> {
        check_target(&self.descriptor, target)?;
        check_capability(self.module.as_ref(), target)?;
        Ok(self
            .module
            .observation_summary(&self.module.observe(target)?))
    }
    fn observe_resources(
        &self,
        target: &TargetIdentity,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, CatalogError> {
        check_target(&self.descriptor, target)?;
        if resources.is_empty()
            || !distinct(resources.iter())
            || resources.iter().any(|key| {
                !self.descriptor.resources.iter().any(|declared| {
                    declared.field == key.field
                        && target.resource_target(declared.scope).as_ref() == Some(&key.target)
                })
            })
        {
            return Err(CatalogError::InvalidResourceSubset);
        }
        check_capability(self.module.as_ref(), target)?;
        let observed =
            self.module
                .observation_states(target, &self.module.observe(target)?, resources)?;
        check_observations(&observed, resources)?;
        Ok(observed)
    }
    fn prepare(
        &self,
        envelope: &ModuleEnvelope,
        target: TargetIdentity,
        mode: ActivationMode,
    ) -> Result<PreparedOperation, CatalogError> {
        if self.descriptor.access == ModuleAccess::ReadOnly {
            return Err(CatalogError::ReadOnly);
        }
        check_target(&self.descriptor, &target)?;
        if !self.descriptor.activation_modes.contains(&mode) {
            return Err(CatalogError::WrongActivation);
        }
        check_capability(self.module.as_ref(), &target)?;
        let config = self.decode_config(envelope)?;
        let current = self.module.observe(&target)?;
        let plan = self.module.prepare(&target, &config, &current)?;
        check_plan(&self.descriptor, &target, &plan)?;
        self.bind(Some(config), plan)
    }
    fn prepare_restore(
        &self,
        envelope: &ModuleEnvelope,
        target: TargetIdentity,
        released: Option<&[ResourceKey]>,
    ) -> Result<PreparedOperation, CatalogError> {
        if self.descriptor.access == ModuleAccess::ReadOnly {
            return Err(CatalogError::ReadOnly);
        }
        check_target(&self.descriptor, &target)?;
        check_capability(self.module.as_ref(), &target)?;
        if envelope.kind != EnvelopeKind::Snapshot {
            return Err(CodecError::WrongKind.into());
        }
        if envelope.target.as_ref() != Some(&target) {
            return Err(CodecError::WrongTarget.into());
        }
        let snapshot = self.module.snapshot_codec().decode(envelope)?;
        let claims = self.module.snapshot_claims(&target, &snapshot)?;
        check_claims(&self.descriptor, &target, &claims)?;
        let all = keys(&claims);
        let released = released.unwrap_or(&all);
        check_subset(&claims, released)?;
        let current = self.module.observe(&target)?;
        if !released.is_empty()
            && self.module.classify(&snapshot, &current, released)? == StateMatch::ExternalChange
        {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "restore preparation").into());
        }
        let plan = self
            .module
            .prepare_restore(&target, &snapshot, &current, released)?;
        check_plan(&self.descriptor, &target, &plan)?;
        if !same_keys(&plan.claims, released)
            || !self.module.same_baseline(&snapshot, &plan.snapshot)
        {
            return Err(CatalogError::InvalidPlan(
                "restore changed subset or first baseline",
            ));
        }
        self.bind(None, plan)
    }
}
fn same_keys(claims: &[ResourceClaim], expected: &[ResourceKey]) -> bool {
    claims.len() == expected.len() && claims.iter().all(|claim| expected.contains(&claim.key))
}
fn check_observations(
    observed: &[ResourceObservation],
    expected: &[ResourceKey],
) -> Result<(), CatalogError> {
    if observed.len() != expected.len()
        || !distinct(observed.iter().map(|state| &state.key))
        || observed.iter().any(|state| !expected.contains(&state.key))
    {
        return Err(CatalogError::InvalidPlan("invalid native observation keys"));
    }
    Ok(())
}

#[derive(Clone, Default)]
pub struct Catalog {
    modules: BTreeMap<FeatureId, Arc<dyn ErasedModule>>,
}
impl Catalog {
    pub(super) fn recover_source(
        &self,
        snapshot: &ModuleEnvelope,
        config: Option<&ModuleEnvelope>,
    ) -> Result<PreparedOperation, CatalogError> {
        self.module(&snapshot.feature)?
            .recover_source(snapshot, config)
    }
    pub fn register<M: Module>(&mut self, module: M) -> Result<(), CatalogError> {
        let descriptor = module.descriptor();
        check_descriptor(&module, &descriptor)?;
        if self.modules.contains_key(&descriptor.id) {
            return Err(CatalogError::DuplicateFeature(descriptor.id));
        }
        self.modules.insert(
            descriptor.id.clone(),
            Arc::new(ModuleAdapter {
                module: Arc::new(module),
                descriptor,
            }),
        );
        Ok(())
    }
    pub fn descriptors(&self) -> impl Iterator<Item = &ModuleDescriptor> {
        self.modules.values().map(|module| module.descriptor())
    }
    fn module(&self, id: &FeatureId) -> Result<&dyn ErasedModule, CatalogError> {
        self.modules
            .get(id)
            .map(|module| module.as_ref())
            .ok_or_else(|| CatalogError::UnknownFeature(id.clone()))
    }
    pub fn validate_config(&self, envelope: &ModuleEnvelope) -> Result<(), CatalogError> {
        self.module(&envelope.feature)?.validate_config(envelope)
    }
    pub fn observe(
        &self,
        feature: &FeatureId,
        target: &TargetIdentity,
    ) -> Result<ObservationSummary, CatalogError> {
        self.module(feature)?.observe(target)
    }
    pub fn observe_resources(
        &self,
        feature: &FeatureId,
        target: &TargetIdentity,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, CatalogError> {
        self.module(feature)?.observe_resources(target, resources)
    }
    pub fn prepare(
        &self,
        config: &ModuleEnvelope,
        target: TargetIdentity,
        mode: ActivationMode,
    ) -> Result<PreparedOperation, CatalogError> {
        self.module(&config.feature)?.prepare(config, target, mode)
    }
    pub fn prepare_restore(
        &self,
        snapshot: &ModuleEnvelope,
        target: TargetIdentity,
    ) -> Result<PreparedOperation, CatalogError> {
        self.module(&snapshot.feature)?
            .prepare_restore(snapshot, target, None)
    }
    pub fn prepare_restore_resources(
        &self,
        snapshot: &ModuleEnvelope,
        target: TargetIdentity,
        released: &[ResourceKey],
    ) -> Result<PreparedOperation, CatalogError> {
        self.module(&snapshot.feature)?
            .prepare_restore(snapshot, target, Some(released))
    }
}

pub(super) struct StepExecutionError {
    pub error: CatalogError,
    pub native_started: bool,
}
impl StepExecutionError {
    fn not_started(error: CatalogError) -> Self {
        Self {
            error,
            native_started: false,
        }
    }
}
trait ErasedOperation: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn descriptor(&self) -> &ModuleDescriptor;
    fn target(&self) -> &TargetIdentity;
    fn claims(&self) -> &[ResourceClaim];
    fn effect(&self) -> Effect;
    fn is_restoration(&self) -> bool;
    fn restores_baseline(&self) -> bool;
    fn mark_cleanup(&mut self);
    fn step_count(&self) -> usize;
    fn step_resources(&self, index: usize) -> Result<&[ResourceKey], CatalogError>;
    fn same_request(&self, other: &dyn ErasedOperation) -> bool;
    fn same_snapshot(&self, other: &dyn ErasedOperation) -> bool;
    fn same_baseline(&self, other: &dyn ErasedOperation) -> bool;
    fn snapshot_envelope(&self) -> Result<ModuleEnvelope, CatalogError>;
    fn config_envelope(&self) -> Result<ModuleEnvelope, CatalogError>;
    fn baseline_states(&self) -> Result<Vec<ResourceObservation>, CatalogError>;
    fn classify(&self, resources: &[ResourceKey]) -> Result<StateMatch, CatalogError>;
    fn observe_resources(
        &self,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, CatalogError>;
    fn execute_step(&self, index: usize) -> Result<Effect, CatalogError>;
    fn verify_step(&self, index: usize) -> Result<Verification, CatalogError>;
    fn step_after(&self, index: usize) -> Result<&[ResourceObservation], CatalogError>;
    fn verify_evidence(
        &self,
        index: usize,
    ) -> Result<(Verification, Vec<ResourceObservation>), CatalogError>;
    fn classified_evidence(
        &self,
        resources: &[ResourceKey],
    ) -> Result<Vec<(ResourceObservation, StateMatch)>, CatalogError>;
    fn update(&self, config: &ModuleEnvelope) -> Result<PreparedOperation, CatalogError>;
    fn restore(&self, released: &[ResourceKey]) -> Result<PreparedOperation, CatalogError>;
    fn restore_expected(
        &self,
        released: &[ResourceKey],
        expected: &ExpectedCurrentProof,
    ) -> Result<PreparedOperation, CatalogError>;
    fn update_expected(
        &self,
        config: &ModuleEnvelope,
        expected: &ExpectedCurrentProof,
    ) -> Result<PreparedOperation, CatalogError>;
    fn execute_expected(
        &self,
        step: usize,
        expected: &ExpectedCurrentProof,
    ) -> Result<Effect, StepExecutionError>;
}
struct TypedOperation<M: Module> {
    module: Arc<M>,
    descriptor: ModuleDescriptor,
    config: Option<M::Config>,
    cleanup: bool,
    plan: Plan<M::Snapshot, M::Step>,
}
impl<M: Module> TypedOperation<M> {
    fn prove_current(
        &self,
        current: &M::Observation,
        resources: &[ResourceKey],
        expected: &ExpectedCurrentProof,
    ) -> Result<(), CatalogError> {
        check_subset(&self.plan.claims, resources)?;
        check_observations(expected.states(), resources)?;
        let observed = self
            .module
            .observation_states(&self.plan.target, current, resources)?;
        check_observations(&observed, resources)?;
        if observed
            .iter()
            .any(|state| !expected.states().contains(state))
        {
            return Err(ModuleError::new(
                ErrorKind::ExternalChange,
                "stale expected-current proof",
            )
            .into());
        }
        Ok(())
    }
    fn peer<'a>(&self, other: &'a dyn ErasedOperation) -> Option<&'a Self> {
        other.as_any().downcast_ref::<Self>().filter(|other| {
            Arc::ptr_eq(&self.module, &other.module) && self.plan.target == other.plan.target
        })
    }
    fn observation(&self) -> Result<M::Observation, CatalogError> {
        check_capability(self.module.as_ref(), &self.plan.target)?;
        Ok(self.module.observe(&self.plan.target)?)
    }
    fn bind(
        &self,
        config: Option<M::Config>,
        plan: Plan<M::Snapshot, M::Step>,
    ) -> Result<PreparedOperation, CatalogError> {
        check_plan(&self.descriptor, &self.plan.target, &plan)?;
        check_snapshot(
            self.module.as_ref(),
            &self.descriptor,
            &plan,
            config.is_some(),
        )?;
        if !self
            .module
            .same_baseline(&self.plan.snapshot, &plan.snapshot)
        {
            return Err(CatalogError::InvalidPlan("first baseline replaced"));
        }
        Ok(PreparedOperation {
            inner: Box::new(Self {
                module: Arc::clone(&self.module),
                descriptor: self.descriptor.clone(),
                cleanup: config.is_none(),
                config,
                plan,
            }),
        })
    }
}
impl<M: Module> ErasedOperation for TypedOperation<M> {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn target(&self) -> &TargetIdentity {
        &self.plan.target
    }
    fn claims(&self) -> &[ResourceClaim] {
        &self.plan.claims
    }
    fn effect(&self) -> Effect {
        self.plan.effect
    }
    fn is_restoration(&self) -> bool {
        self.cleanup
    }
    fn restores_baseline(&self) -> bool {
        self.config.is_none()
    }
    fn mark_cleanup(&mut self) {
        self.cleanup = true;
    }
    fn step_count(&self) -> usize {
        self.plan.steps.len()
    }
    fn step_resources(&self, index: usize) -> Result<&[ResourceKey], CatalogError> {
        self.plan
            .steps
            .get(index)
            .map(|step| step.writes.as_slice())
            .ok_or(CatalogError::StepOutOfBounds)
    }
    fn same_request(&self, other: &dyn ErasedOperation) -> bool {
        self.peer(other)
            .is_some_and(|other| self.config.is_some() && self.config == other.config)
    }
    fn same_snapshot(&self, other: &dyn ErasedOperation) -> bool {
        self.peer(other)
            .is_some_and(|other| self.plan.snapshot == other.plan.snapshot)
    }
    fn same_baseline(&self, other: &dyn ErasedOperation) -> bool {
        self.peer(other).is_some_and(|other| {
            self.module
                .same_baseline(&self.plan.snapshot, &other.plan.snapshot)
        })
    }
    fn snapshot_envelope(&self) -> Result<ModuleEnvelope, CatalogError> {
        Ok(self
            .module
            .snapshot_codec()
            .encode(&self.plan.snapshot, Some(&self.plan.target))?)
    }
    fn config_envelope(&self) -> Result<ModuleEnvelope, CatalogError> {
        self.module
            .config_codec()
            .encode(self.config.as_ref().ok_or(CatalogError::ReadOnly)?, None)
            .map_err(Into::into)
    }
    fn baseline_states(&self) -> Result<Vec<ResourceObservation>, CatalogError> {
        let resources = keys(&self.plan.claims);
        let states =
            self.module
                .baseline_states(&self.plan.target, &self.plan.snapshot, &resources)?;
        check_observations(&states, &resources)?;
        Ok(states)
    }
    fn classify(&self, resources: &[ResourceKey]) -> Result<StateMatch, CatalogError> {
        check_subset(&self.plan.claims, resources)?;
        if resources.is_empty() {
            return Ok(StateMatch::Baseline);
        }
        Ok(self
            .module
            .classify(&self.plan.snapshot, &self.observation()?, resources)?)
    }
    fn observe_resources(
        &self,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, CatalogError> {
        check_subset(&self.plan.claims, resources)?;
        let observed =
            self.module
                .observation_states(&self.plan.target, &self.observation()?, resources)?;
        check_observations(&observed, resources)?;
        Ok(observed)
    }
    fn execute_step(&self, index: usize) -> Result<Effect, CatalogError> {
        let step = self
            .plan
            .steps
            .get(index)
            .ok_or(CatalogError::StepOutOfBounds)?;
        if self.classify(&step.writes)? == StateMatch::ExternalChange {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "step preflight").into());
        }
        Ok(self
            .module
            .execute_step(&self.plan.target, &step.instruction)?)
    }
    fn verify_step(&self, index: usize) -> Result<Verification, CatalogError> {
        let step = self
            .plan
            .steps
            .get(index)
            .ok_or(CatalogError::StepOutOfBounds)?;
        Ok(self
            .module
            .verify_step(&self.plan.target, &step.instruction, &self.observation()?)?)
    }
    fn step_after(&self, index: usize) -> Result<&[ResourceObservation], CatalogError> {
        self.plan
            .steps
            .get(index)
            .map(|step| step.after.as_slice())
            .ok_or(CatalogError::StepOutOfBounds)
    }
    fn verify_evidence(
        &self,
        index: usize,
    ) -> Result<(Verification, Vec<ResourceObservation>), CatalogError> {
        let step = self
            .plan
            .steps
            .get(index)
            .ok_or(CatalogError::StepOutOfBounds)?;
        let current = self.observation()?;
        let verification =
            self.module
                .verify_step(&self.plan.target, &step.instruction, &current)?;
        let states = self
            .module
            .observation_states(&self.plan.target, &current, &step.writes)?;
        check_observations(&states, &step.writes)?;
        Ok((verification, states))
    }
    fn classified_evidence(
        &self,
        resources: &[ResourceKey],
    ) -> Result<Vec<(ResourceObservation, StateMatch)>, CatalogError> {
        check_subset(&self.plan.claims, resources)?;
        let current = self.observation()?;
        let states = self
            .module
            .observation_states(&self.plan.target, &current, resources)?;
        check_observations(&states, resources)?;
        states
            .into_iter()
            .map(|state| {
                let classification = self.module.classify(
                    &self.plan.snapshot,
                    &current,
                    std::slice::from_ref(&state.key),
                )?;
                Ok((state, classification))
            })
            .collect()
    }
    fn update(&self, config: &ModuleEnvelope) -> Result<PreparedOperation, CatalogError> {
        let config = self.module.config_codec().decode(config)?;
        self.module.validate_config(&config)?;
        let current = self.observation()?;
        if self
            .module
            .classify(&self.plan.snapshot, &current, &keys(&self.plan.claims))?
            == StateMatch::ExternalChange
        {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "update preparation").into());
        }
        let plan = self.module.prepare_update(
            &self.plan.target,
            &config,
            &self.plan.snapshot,
            &current,
        )?;
        if !same_keys(&plan.claims, &keys(&self.plan.claims)) {
            return Err(CatalogError::InvalidPlan("update changed resource set"));
        }
        self.bind(Some(config), plan)
    }
    fn restore(&self, released: &[ResourceKey]) -> Result<PreparedOperation, CatalogError> {
        check_subset(&self.plan.claims, released)?;
        let current = self.observation()?;
        if !released.is_empty()
            && self
                .module
                .classify(&self.plan.snapshot, &current, released)?
                == StateMatch::ExternalChange
        {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "restore preparation").into());
        }
        let plan = self.module.prepare_restore(
            &self.plan.target,
            &self.plan.snapshot,
            &current,
            released,
        )?;
        if !same_keys(&plan.claims, released) {
            return Err(CatalogError::InvalidPlan("restore changed resource subset"));
        }
        self.bind(None, plan)
    }
    fn restore_expected(
        &self,
        released: &[ResourceKey],
        expected: &ExpectedCurrentProof,
    ) -> Result<PreparedOperation, CatalogError> {
        let current = self.observation()?;
        self.prove_current(&current, released, expected)?;
        let plan = self.module.prepare_restore(
            &self.plan.target,
            &self.plan.snapshot,
            &current,
            released,
        )?;
        if !same_keys(&plan.claims, released) {
            return Err(CatalogError::InvalidPlan("restore changed resource subset"));
        }
        self.bind(None, plan)
    }
    fn update_expected(
        &self,
        config: &ModuleEnvelope,
        expected: &ExpectedCurrentProof,
    ) -> Result<PreparedOperation, CatalogError> {
        let config = self.module.config_codec().decode(config)?;
        self.module.validate_config(&config)?;
        let current = self.observation()?;
        self.prove_current(&current, &keys(&self.plan.claims), expected)?;
        let plan = self.module.prepare_update(
            &self.plan.target,
            &config,
            &self.plan.snapshot,
            &current,
        )?;
        if !same_keys(&plan.claims, &keys(&self.plan.claims)) {
            return Err(CatalogError::InvalidPlan("update changed resource set"));
        }
        let mut prepared = self.bind(Some(config), plan)?;
        prepared.inner.mark_cleanup();
        Ok(prepared)
    }
    fn execute_expected(
        &self,
        index: usize,
        expected: &ExpectedCurrentProof,
    ) -> Result<Effect, StepExecutionError> {
        let step = self
            .plan
            .steps
            .get(index)
            .ok_or(CatalogError::StepOutOfBounds)
            .map_err(StepExecutionError::not_started)?;
        let observed = self
            .observation()
            .map_err(StepExecutionError::not_started)?;
        self.prove_current(&observed, &step.writes, expected)
            .map_err(StepExecutionError::not_started)?;
        self.module
            .execute_step(&self.plan.target, &step.instruction)
            .map_err(|error| StepExecutionError {
                error: error.into(),
                native_started: true,
            })
    }
}

/// Executor identity and concrete state stay bound together. There is no API to
/// supply another module's step or to inspect a generic JSON runtime plan.
pub struct PreparedOperation {
    inner: Box<dyn ErasedOperation>,
}
impl PreparedOperation {
    pub(super) fn restores_baseline(&self) -> bool {
        self.inner.restores_baseline()
    }
    pub(super) fn step_after(&self, step: usize) -> Result<&[ResourceObservation], CatalogError> {
        self.inner.step_after(step)
    }
    pub(super) fn verify_step_evidence(
        &self,
        step: usize,
    ) -> Result<(Verification, Vec<ResourceObservation>), CatalogError> {
        self.inner.verify_evidence(step)
    }
    pub(super) fn observe_classified(
        &self,
        resources: &[ResourceKey],
    ) -> Result<Vec<(ResourceObservation, StateMatch)>, CatalogError> {
        self.inner.classified_evidence(resources)
    }
    pub(in crate::app::features::tuning) fn config_envelope(
        &self,
    ) -> Result<ModuleEnvelope, CatalogError> {
        self.inner.config_envelope()
    }
    pub(in crate::app::features::tuning) fn baseline_states(
        &self,
    ) -> Result<Vec<ResourceObservation>, CatalogError> {
        self.inner.baseline_states()
    }
    pub(super) fn prepare_restore_expected(
        &self,
        resources: &[ResourceKey],
        expected: &ExpectedCurrentProof,
    ) -> Result<Self, CatalogError> {
        self.inner.restore_expected(resources, expected)
    }
    pub(super) fn prepare_update_expected(
        &self,
        config: &ModuleEnvelope,
        expected: &ExpectedCurrentProof,
    ) -> Result<Self, CatalogError> {
        self.inner.update_expected(config, expected)
    }
    pub(super) fn execute_step_expected(
        &self,
        step: usize,
        expected: &ExpectedCurrentProof,
    ) -> Result<Effect, StepExecutionError> {
        self.inner.execute_expected(step, expected)
    }
    pub fn is_restoration(&self) -> bool {
        self.inner.is_restoration()
    }
    pub fn descriptor(&self) -> &ModuleDescriptor {
        self.inner.descriptor()
    }
    pub fn target(&self) -> &TargetIdentity {
        self.inner.target()
    }
    pub fn resources(&self) -> &[ResourceClaim] {
        self.inner.claims()
    }
    pub fn effect(&self) -> Effect {
        self.inner.effect()
    }
    pub fn step_count(&self) -> usize {
        self.inner.step_count()
    }
    pub fn step_resources(&self, index: usize) -> Result<&[ResourceKey], CatalogError> {
        self.inner.step_resources(index)
    }
    pub fn same_request(&self, other: &Self) -> bool {
        self.inner.same_request(other.inner.as_ref())
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.inner.same_snapshot(other.inner.as_ref())
    }
    pub fn same_baseline(&self, other: &Self) -> bool {
        self.inner.same_baseline(other.inner.as_ref())
    }
    pub fn snapshot_envelope(&self) -> Result<ModuleEnvelope, CatalogError> {
        self.inner.snapshot_envelope()
    }
    pub(in crate::app::features::tuning) fn classify(&self) -> Result<StateMatch, CatalogError> {
        self.inner.classify(&keys(self.resources()))
    }
    pub(in crate::app::features::tuning) fn classify_resources(
        &self,
        resources: &[ResourceKey],
    ) -> Result<StateMatch, CatalogError> {
        self.inner.classify(resources)
    }
    pub(in crate::app::features::tuning) fn observe_resources(
        &self,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, CatalogError> {
        self.inner.observe_resources(resources)
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn execute_step(
        &self,
        index: usize,
    ) -> Result<Effect, CatalogError> {
        self.inner.execute_step(index)
    }
    pub(in crate::app::features::tuning) fn verify_step(
        &self,
        index: usize,
    ) -> Result<Verification, CatalogError> {
        self.inner.verify_step(index)
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn prepare_update(
        &self,
        config: &ModuleEnvelope,
    ) -> Result<Self, CatalogError> {
        self.inner.update(config)
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn prepare_restore(
        &self,
        released: &[ResourceKey],
    ) -> Result<Self, CatalogError> {
        self.inner.restore(released)
    }
}
