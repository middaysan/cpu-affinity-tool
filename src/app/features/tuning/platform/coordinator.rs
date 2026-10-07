//! Synchronous ownership engine. Only the serial runner dispatches commands.
use super::{
    catalog::{Catalog, CatalogError, PreparedOperation},
    codec::ModuleEnvelope,
    contract::*,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
pub enum Command {
    Acquire {
        owner: OwnerId,
        config: ModuleEnvelope,
        target: TargetIdentity,
        mode: ActivationMode,
    },
    Release {
        owner: OwnerId,
    },
    Observe {
        feature: FeatureId,
        target: TargetIdentity,
    },
    /// An authoritative lifecycle producer attests this exact process/logon has
    /// ended. Shape validation, denied reads, rule deletion and UI intent are
    /// insufficient. F4 supplies that producer; F2 has no native lifetime input.
    Retire {
        target: TargetIdentity,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateError {
    MutationDisabled,
    Cancelled,
    Rejected(&'static str),
    Durability(super::journal::JournalError),
}
pub(super) struct WriteIntent<'a> {
    pub operation: &'a PreparedOperation,
    pub step: usize,
    pub ownership: CoordinatorSnapshot,
}
pub(super) trait ExecutionControl {
    fn cancellation_requested(&self) -> bool;
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError>;
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError>;
    fn publish(&mut self, snapshot: CoordinatorSnapshot);
    fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoordinatorError {
    Catalog(CatalogError),
    Conflict(Vec<ResourceKey>),
    Gate(GateError),
    Verification,
    ExternalDrift(Vec<ResourceKey>),
    ResourceSetChanged,
    Capacity,
    EffectMismatch { declared: Effect, actual: Effect },
}
impl From<CatalogError> for CoordinatorError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Applied(Effect),
    Shared,
    Unchanged,
    Released,
    Observed(ObservationSummary),
    Cancelled,
    Compensated,
    RecoveryRequired,
    Rejected,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceView {
    pub key: ResourceKey,
    pub desired: DesiredStateToken,
    pub baseline: DesiredStateToken,
    pub owners: Vec<OwnerId>,
    pub status: ModuleStatus,
    pub baseline_source: FeatureId,
    pub baseline_snapshot: Arc<ModuleEnvelope>,
    pub baseline_config: Option<Arc<ModuleEnvelope>>,
    pub current_source: FeatureId,
    pub current_snapshot: Arc<ModuleEnvelope>,
    pub current_config: Option<Arc<ModuleEnvelope>>,
    pub confirmed: DesiredStateToken,
    pub possible: Vec<DesiredStateToken>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerView {
    pub owner: OwnerId,
    pub resources: Vec<ResourceKey>,
    pub status: ModuleStatus,
    pub mode: ActivationMode,
    pub source: FeatureId,
    pub target: TargetIdentity,
    pub config: Option<Arc<ModuleEnvelope>>,
    pub snapshot: Arc<ModuleEnvelope>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoordinatorSnapshot {
    pub resources: Vec<ResourceView>,
    pub owners: Vec<OwnerView>,
    pub restoration_effect: Option<Effect>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandResult {
    pub outcome: Outcome,
    pub error: Option<CoordinatorError>,
    pub snapshot: CoordinatorSnapshot,
    pub restoration_effect: Option<Effect>,
}

struct Source {
    operation: Arc<PreparedOperation>,
    snapshot: Arc<ModuleEnvelope>,
    config: Option<Arc<ModuleEnvelope>>,
}
impl Source {
    fn new(operation: PreparedOperation) -> Result<Arc<Self>, CoordinatorError> {
        let snapshot = operation.snapshot_envelope()?;
        let config = match operation.config_envelope() {
            Ok(envelope) => Some(Arc::new(envelope)),
            Err(CatalogError::ReadOnly) => None,
            Err(error) => return Err(error.into()),
        };
        Ok(Arc::new(Self {
            operation: Arc::new(operation),
            snapshot: Arc::new(snapshot),
            config,
        }))
    }
}
#[derive(Clone)]
struct ResourceRecord {
    original: Arc<Source>,
    current: Arc<Source>,
    baseline: DesiredStateToken,
    desired: DesiredStateToken,
    confirmed: DesiredStateToken,
    possible: Vec<DesiredStateToken>,
    owners: BTreeSet<OwnerId>,
    status: ModuleStatus,
}
#[derive(Clone)]
struct OwnerRecord {
    source: Arc<Source>,
    mode: ActivationMode,
    resources: Vec<ResourceKey>,
    status: ModuleStatus,
}
#[derive(Clone)]
struct LedgerCheckpoint {
    resources: BTreeMap<ResourceKey, ResourceRecord>,
    owners: BTreeMap<OwnerId, OwnerRecord>,
    restoration_effect: Option<Effect>,
}
const MAX_OWNERS: usize = 256;
const MAX_RESOURCES: usize = 4096;

pub struct Coordinator {
    catalog: Catalog,
    resources: BTreeMap<ResourceKey, ResourceRecord>,
    owners: BTreeMap<OwnerId, OwnerRecord>,
    restoration_effect: Option<Effect>,
    command_native_started: bool,
    durable_checkpoint: Option<LedgerCheckpoint>,
    durability_failed: bool,
}
impl Coordinator {
    pub(super) fn recover(
        catalog: Catalog,
        document: &super::journal::JournalDocument,
    ) -> Result<Self, CoordinatorError> {
        document
            .validate()
            .map_err(|error| CoordinatorError::Gate(GateError::Durability(error)))?;
        let mut sources = Vec::with_capacity(document.sources.len());
        for source in &document.sources {
            sources.push(Source::new(
                catalog.recover_source(&source.snapshot, source.config.as_ref())?,
            )?);
        }
        let mut recovered = Self::new(catalog);
        recovered.restoration_effect = document.restoration_effect;
        for stored in &document.resources {
            let original = Arc::clone(&sources[stored.original.0 as usize]);
            let current = Arc::clone(&sources[stored.current.0 as usize]);
            if original.config.is_none()
                || !original
                    .operation
                    .baseline_states()?
                    .iter()
                    .any(|state| state.key == stored.key && state.value == stored.baseline)
            {
                return Err(CatalogError::InvalidPlan("journal first baseline mismatch").into());
            }
            for source in [&original, &current] {
                if !source.operation.resources().iter().any(|claim| {
                    claim.key == stored.key
                        && claim.desired.semantics == stored.desired.semantics
                        && claim.desired.version == stored.desired.version
                }) {
                    return Err(CatalogError::InvalidPlan("journal source claims mismatch").into());
                }
            }
            recovered.resources.insert(
                stored.key.clone(),
                ResourceRecord {
                    original,
                    current,
                    baseline: stored.baseline.clone(),
                    desired: stored.desired.clone(),
                    confirmed: stored.confirmed.clone(),
                    possible: stored.attempted.clone(),
                    owners: stored.owners.iter().copied().collect(),
                    status: stored.status,
                },
            );
        }
        for stored in &document.owners {
            let source = Arc::clone(&sources[stored.source.0 as usize]);
            if !source
                .operation
                .descriptor()
                .activation_modes
                .contains(&stored.mode)
                || stored.resources.iter().any(|key| {
                    !source.operation.resources().iter().any(|claim| {
                        claim.key == *key && claim.desired == recovered.resources[key].desired
                    })
                })
            {
                return Err(CatalogError::InvalidPlan("journal owner intent mismatch").into());
            }
            recovered.owners.insert(
                stored.owner,
                OwnerRecord {
                    source,
                    mode: stored.mode,
                    resources: stored.resources.clone(),
                    status: stored.status,
                },
            );
        }
        Ok(recovered)
    }
    pub(super) fn apply_recovery(
        &mut self,
        plan: &super::recovery::RecoveryPlan,
        control: &mut dyn ExecutionControl,
    ) -> CommandResult {
        if plan.blocked() || self.durability_failed {
            return self.result(
                Outcome::RecoveryRequired,
                Some(CoordinatorError::Gate(GateError::Durability(
                    plan.error
                        .clone()
                        .unwrap_or(super::journal::JournalError::Blocked),
                ))),
                control,
            );
        }
        let covered = plan
            .reattach
            .iter()
            .chain(&plan.release)
            .copied()
            .collect::<BTreeSet<_>>();
        if covered.len() != plan.reattach.len() + plan.release.len()
            || covered != self.owners.keys().copied().collect()
            || plan.retire.iter().any(|target| {
                !matches!(
                    target,
                    TargetIdentity::Process(_) | TargetIdentity::Session(_)
                )
            })
        {
            return self.result(
                Outcome::RecoveryRequired,
                Some(CoordinatorError::ResourceSetChanged),
                control,
            );
        }
        let before = self.checkpoint();
        let before_snapshot = self.snapshot();
        self.command_native_started = false;
        self.durable_checkpoint = Some(before.clone());
        let mut error = None;
        // A native producer proves attachment; setting readback proves only
        // the exact current value. Never promote another historical or planned
        // snapshot value to new app-written evidence.
        let active = plan.reattach.iter().copied().collect::<BTreeSet<_>>();
        let keys = self
            .resources
            .iter()
            .filter(|(key, record)| {
                record.owners.iter().any(|owner| active.contains(owner))
                    && !plan
                        .retire
                        .iter()
                        .any(|ended| ended_lifetime(&key.target, ended))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            let source = Arc::clone(&self.resources[&key].current);
            match source
                .operation
                .observe_resources(std::slice::from_ref(&key))
            {
                Ok(states) => {
                    let state = &states[0];
                    let record = self.resources.get_mut(&key).unwrap();
                    let evidenced =
                        state.value == record.confirmed || record.possible.contains(&state.value);
                    if state.value == record.desired && evidenced {
                        record.confirmed = state.value.clone();
                        record.possible.clear();
                        record.status = active_status(source.operation.effect());
                    } else if evidenced || state.value == record.baseline {
                        record.status = ModuleStatus::RecoveryRequired;
                    } else {
                        record.status = ModuleStatus::ExternalChange;
                        error = Some(CoordinatorError::ExternalDrift(vec![key.clone()]));
                    }
                }
                Err(failure) => {
                    let failure = CoordinatorError::from(failure);
                    let owners = self.resources[&key]
                        .owners
                        .iter()
                        .copied()
                        .collect::<Vec<_>>();
                    for owner in owners {
                        self.mark_pending(owner, std::slice::from_ref(&key), &failure);
                    }
                    error = Some(failure);
                }
            }
        }
        for owner in &plan.reattach {
            if let Some(request) = self.owners.get_mut(owner) {
                request.status = if request.resources.iter().any(|key| {
                    self.resources
                        .get(key)
                        .is_some_and(|record| unresolved(record.status))
                }) {
                    ModuleStatus::RecoveryRequired
                } else {
                    active_status(request.source.operation.effect())
                };
            }
        }
        // Expiry concerns actual resource keys; the original adapter may remain
        // bound to a dead activation while restoring a live Machine/User field.
        for ended in &plan.retire {
            let result = self.retire(ended, control);
            if result.error.is_some() {
                error = result.error;
            }
        }
        for owner in &plan.release {
            let result = self.release(*owner, control);
            if result.error.is_some() {
                error = result.error;
            }
        }
        let unresolved = self
            .resources
            .values()
            .any(|record| unresolved(record.status));
        let result = self.result(
            if unresolved {
                Outcome::RecoveryRequired
            } else if self.owners.is_empty() && !covered.is_empty() {
                Outcome::Released
            } else {
                Outcome::Unchanged
            },
            error,
            control,
        );
        self.finish_transition(result, before, before_snapshot, control)
    }
    pub fn new(catalog: Catalog) -> Self {
        Self {
            catalog,
            resources: BTreeMap::new(),
            owners: BTreeMap::new(),
            restoration_effect: None,
            command_native_started: false,
            durable_checkpoint: None,
            durability_failed: false,
        }
    }
    pub fn snapshot(&self) -> CoordinatorSnapshot {
        CoordinatorSnapshot {
            restoration_effect: self.restoration_effect,
            resources: self
                .resources
                .iter()
                .map(|(key, record)| ResourceView {
                    key: key.clone(),
                    desired: record.desired.clone(),
                    baseline: record.baseline.clone(),
                    owners: record.owners.iter().copied().collect(),
                    status: record.status,
                    baseline_source: record.original.operation.descriptor().id.clone(),
                    baseline_snapshot: record.original.snapshot.clone(),
                    baseline_config: record.original.config.clone(),
                    current_source: record.current.operation.descriptor().id.clone(),
                    current_snapshot: record.current.snapshot.clone(),
                    current_config: record.current.config.clone(),
                    confirmed: record.confirmed.clone(),
                    possible: record.possible.clone(),
                })
                .collect(),
            owners: self
                .owners
                .iter()
                .map(|(owner, record)| OwnerView {
                    owner: *owner,
                    resources: record.resources.clone(),
                    status: record.status,
                    mode: record.mode,
                    source: record.source.operation.descriptor().id.clone(),
                    target: record.source.operation.target().clone(),
                    config: record.source.config.clone(),
                    snapshot: record.source.snapshot.clone(),
                })
                .collect(),
        }
    }
    fn result(
        &self,
        outcome: Outcome,
        error: Option<CoordinatorError>,
        control: &mut dyn ExecutionControl,
    ) -> CommandResult {
        let snapshot = self.snapshot();
        control.publish(snapshot.clone());
        CommandResult {
            outcome,
            error,
            snapshot,
            restoration_effect: self.restoration_effect,
        }
    }
    pub(super) fn handle(
        &mut self,
        command: Command,
        control: &mut dyn ExecutionControl,
    ) -> CommandResult {
        if matches!(command, Command::Observe { .. }) {
            return self.dispatch(command, control);
        }
        if self.durability_failed {
            return self.result(
                Outcome::RecoveryRequired,
                Some(CoordinatorError::Gate(GateError::Durability(
                    super::journal::JournalError::Blocked,
                ))),
                control,
            );
        }
        let before = self.checkpoint();
        let before_snapshot = self.snapshot();
        self.command_native_started = false;
        self.durable_checkpoint = None;
        let result = self.dispatch(command, control);
        self.finish_transition(result, before, before_snapshot, control)
    }
    fn dispatch(&mut self, command: Command, control: &mut dyn ExecutionControl) -> CommandResult {
        match command {
            Command::Acquire {
                owner,
                config,
                target,
                mode,
            } => self.acquire(owner, config, target, mode, control),
            Command::Release { owner } => self.release(owner, control),
            Command::Observe { feature, target } => {
                if control.cancellation_requested() {
                    return self.result(Outcome::Cancelled, None, control);
                }
                match self.catalog.observe(&feature, &target) {
                    Ok(observed) => self.result(Outcome::Observed(observed), None, control),
                    Err(error) => self.result(Outcome::Rejected, Some(error.into()), control),
                }
            }
            Command::Retire { target } => self.retire(&target, control),
        }
    }
    pub(super) fn shutdown(&mut self, control: &mut dyn ExecutionControl) -> CommandResult {
        if self.durability_failed {
            return self.result(
                Outcome::RecoveryRequired,
                Some(CoordinatorError::Gate(GateError::Durability(
                    super::journal::JournalError::Blocked,
                ))),
                control,
            );
        }
        let before = self.checkpoint();
        let before_snapshot = self.snapshot();
        self.command_native_started = false;
        self.durable_checkpoint = None;
        let result = self.shutdown_inner(control);
        self.finish_transition(result, before, before_snapshot, control)
    }
    fn shutdown_inner(&mut self, control: &mut dyn ExecutionControl) -> CommandResult {
        let session_owners = self
            .owners
            .iter()
            .filter(|(_, record)| record.mode != ActivationMode::Manual)
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        let mut error = None;
        for owner in session_owners {
            let result = self.release(owner, control);
            if result.error.is_some() {
                error = result.error;
            }
        }
        if self
            .resources
            .values()
            .any(|record| unresolved(record.status))
        {
            self.result(Outcome::RecoveryRequired, error, control)
        } else {
            self.result(Outcome::Released, error, control)
        }
    }
    fn checkpoint(&self) -> LedgerCheckpoint {
        LedgerCheckpoint {
            resources: self.resources.clone(),
            owners: self.owners.clone(),
            restoration_effect: self.restoration_effect,
        }
    }
    fn restore_checkpoint(&mut self, checkpoint: LedgerCheckpoint) {
        self.resources = checkpoint.resources;
        self.owners = checkpoint.owners;
        self.restoration_effect = checkpoint.restoration_effect;
    }
    fn finish_transition(
        &mut self,
        result: CommandResult,
        before: LedgerCheckpoint,
        before_snapshot: CoordinatorSnapshot,
        control: &mut dyn ExecutionControl,
    ) -> CommandResult {
        if self.command_native_started
            || self.durable_checkpoint.is_some()
            || self.snapshot() != before_snapshot
        {
            if let Err(error) = control.commit_transition(&self.snapshot()) {
                if self.command_native_started {
                    let checkpoint = self.durable_checkpoint.take().unwrap_or(before);
                    self.restore_checkpoint(checkpoint);
                    for resource in self.resources.values_mut() {
                        resource.status = ModuleStatus::RecoveryRequired;
                    }
                    for owner in self.owners.values_mut() {
                        owner.status = ModuleStatus::RecoveryRequired;
                    }
                } else {
                    self.restore_checkpoint(before);
                }
                self.durability_failed = true;
                return self.result(
                    if self.command_native_started {
                        Outcome::RecoveryRequired
                    } else {
                        Outcome::Rejected
                    },
                    Some(CoordinatorError::Gate(error)),
                    control,
                );
            }
        }
        result
    }
    fn acquire(
        &mut self,
        owner: OwnerId,
        config: ModuleEnvelope,
        target: TargetIdentity,
        mode: ActivationMode,
        control: &mut dyn ExecutionControl,
    ) -> CommandResult {
        if control.cancellation_requested() {
            return self.result(Outcome::Cancelled, None, control);
        }
        let prepared = match self
            .catalog
            .prepare(&config, target, mode)
            .map_err(CoordinatorError::from)
            .and_then(Source::new)
        {
            Ok(source) => source,
            Err(error) => return self.result(Outcome::Rejected, Some(error), control),
        };
        let claims = prepared.operation.resources();
        let keys = claims
            .iter()
            .map(|claim| claim.key.clone())
            .collect::<Vec<_>>();
        let previous_owner = self.owners.get(&owner).cloned();
        if previous_owner.as_ref().is_some_and(|previous| {
            previous.resources.len() != keys.len()
                || previous.resources.iter().any(|key| !keys.contains(key))
        }) {
            return self.result(
                Outcome::Rejected,
                Some(CoordinatorError::ResourceSetChanged),
                control,
            );
        }
        let conflicts = claims
            .iter()
            .filter(|claim| {
                self.resources.get(&claim.key).is_some_and(|record| {
                    unresolved(record.status)
                        || (record.desired != claim.desired
                            && (record.owners.len() != 1 || !record.owners.contains(&owner)))
                })
            })
            .map(|claim| claim.key.clone())
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            return self.result(
                Outcome::Rejected,
                Some(CoordinatorError::Conflict(conflicts)),
                control,
            );
        }
        let new_count = keys
            .iter()
            .filter(|key| !self.resources.contains_key(*key))
            .count();
        if (previous_owner.is_none() && self.owners.len() >= MAX_OWNERS)
            || self.resources.len() + new_count > MAX_RESOURCES
        {
            return self.result(Outcome::Rejected, Some(CoordinatorError::Capacity), control);
        }
        let baselines = match prepared.operation.baseline_states() {
            Ok(states) => states,
            Err(error) => return self.result(Outcome::Rejected, Some(error.into()), control),
        };
        // Bind new resource baselines to the immutable prepared snapshot, while
        // existing fields must still match retained evidence from their first owner.
        let observed = match prepared.operation.observe_resources(&keys) {
            Ok(states) => states,
            Err(error) => return self.result(Outcome::Rejected, Some(error.into()), control),
        };
        let drift = observed
            .iter()
            .filter(|state| match self.resources.get(&state.key) {
                Some(record) => {
                    state.value != record.confirmed && !record.possible.contains(&state.value)
                }
                None => !baselines.iter().any(|baseline| baseline == *state),
            })
            .map(|state| state.key.clone())
            .collect::<Vec<_>>();
        if !drift.is_empty() {
            let error = CoordinatorError::ExternalDrift(drift);
            self.mark_evidence_failure(&keys, &error);
            return self.result(Outcome::Rejected, Some(error), control);
        }
        if control.cancellation_requested() {
            return self.result(Outcome::Cancelled, None, control);
        }
        let same_values = claims.iter().all(|claim| {
            self.resources
                .get(&claim.key)
                .is_some_and(|record| record.desired == claim.desired)
        });
        if previous_owner.is_some() && same_values {
            let previous = self.owners.get_mut(&owner).unwrap();
            previous.mode = mode;
            previous.source = Arc::clone(&prepared);
            return self.result(Outcome::Unchanged, None, control);
        }
        let previous_records = keys
            .iter()
            .filter_map(|key| {
                self.resources
                    .get(key)
                    .map(|record| (key.clone(), record.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        for claim in claims {
            let record = self.resources.entry(claim.key.clone()).or_insert_with(|| {
                let baseline = baselines
                    .iter()
                    .find(|baseline| baseline.key == claim.key)
                    .unwrap()
                    .value
                    .clone();
                ResourceRecord {
                    original: Arc::clone(&prepared),
                    current: Arc::clone(&prepared),
                    baseline: baseline.clone(),
                    desired: claim.desired.clone(),
                    confirmed: baseline,
                    possible: vec![],
                    owners: BTreeSet::new(),
                    status: ModuleStatus::Applying,
                }
            });
            record.owners.insert(owner);
            if record.desired != claim.desired || !previous_records.contains_key(&claim.key) {
                record.current = Arc::clone(&prepared);
                record.desired = claim.desired.clone();
            }
            record.status = ModuleStatus::Applying;
        }
        self.owners.insert(
            owner,
            OwnerRecord {
                source: Arc::clone(&prepared),
                mode,
                resources: keys.clone(),
                status: ModuleStatus::Applying,
            },
        );
        control.publish(self.snapshot());
        let mut possible_write = false;
        match self.execute(&prepared, control, &mut possible_write) {
            Ok(()) => {
                let status = active_status(prepared.operation.effect());
                for key in &keys {
                    self.resources.get_mut(key).unwrap().status = status;
                }
                self.owners.get_mut(&owner).unwrap().status = status;
                let outcome = if same_values && previous_owner.is_none() {
                    Outcome::Shared
                } else {
                    Outcome::Applied(prepared.operation.effect())
                };
                self.result(outcome, None, control)
            }
            Err(error) => {
                if !possible_write {
                    self.restore_table(owner, &keys, previous_owner, &previous_records);
                    self.mark_evidence_failure(&keys, &error);
                    let outcome = if matches!(error, CoordinatorError::Gate(GateError::Cancelled)) {
                        Outcome::Cancelled
                    } else {
                        Outcome::Rejected
                    };
                    return self.result(outcome, Some(error), control);
                }
                if let Some(previous) = previous_owner {
                    // Restore the preceding logical request, retaining the first
                    // baseline and all evidence of the failed new writer.
                    self.owners.insert(owner, previous.clone());
                    for key in &keys {
                        let record = self.resources.get_mut(key).unwrap();
                        record.desired = previous_records[key].desired.clone();
                    }
                    let cleanup = self.compensate_previous(&previous, control);
                    if cleanup.is_ok() {
                        for key in &keys {
                            self.resources.get_mut(key).unwrap().status = previous.status;
                        }
                        self.result(Outcome::Compensated, Some(error), control)
                    } else {
                        self.mark_pending(owner, &keys, cleanup.as_ref().err().unwrap());
                        self.result(Outcome::RecoveryRequired, Some(error), control)
                    }
                } else {
                    let cleanup = self.release(owner, control);
                    if cleanup.outcome == Outcome::RecoveryRequired {
                        self.result(Outcome::RecoveryRequired, Some(error), control)
                    } else {
                        self.result(Outcome::Compensated, Some(error), control)
                    }
                }
            }
        }
    }
    fn restore_table(
        &mut self,
        owner: OwnerId,
        keys: &[ResourceKey],
        previous_owner: Option<OwnerRecord>,
        records: &BTreeMap<ResourceKey, ResourceRecord>,
    ) {
        for key in keys {
            if let Some(previous) = records.get(key) {
                self.resources.insert(key.clone(), previous.clone());
            } else {
                self.resources.remove(key);
            }
        }
        if let Some(previous) = previous_owner {
            self.owners.insert(owner, previous);
        } else {
            self.owners.remove(&owner);
        }
    }
    fn proof(
        &self,
        source: &Source,
        keys: &[ResourceKey],
        allow_baseline: bool,
    ) -> Result<ExpectedCurrentProof, CoordinatorError> {
        let states = source.operation.observe_resources(keys)?;
        let drift = states
            .iter()
            .filter(|state| {
                self.resources.get(&state.key).is_none_or(|record| {
                    state.value != record.confirmed
                        && !(allow_baseline && state.value == record.baseline)
                        && !record.possible.contains(&state.value)
                })
            })
            .map(|state| state.key.clone())
            .collect::<Vec<_>>();
        if !drift.is_empty() {
            return Err(CoordinatorError::ExternalDrift(drift));
        }
        Ok(ExpectedCurrentProof::new(states))
    }
    fn execute(
        &mut self,
        source: &Arc<Source>,
        control: &mut dyn ExecutionControl,
        possible_write: &mut bool,
    ) -> Result<(), CoordinatorError> {
        let operation = &source.operation;
        for step in 0..operation.step_count() {
            if !operation.is_restoration() && control.cancellation_requested() {
                return Err(CoordinatorError::Gate(GateError::Cancelled));
            }
            let writes = operation.step_resources(step)?.to_vec();
            let proof = self.proof(source, &writes, operation.restores_baseline())?;
            let after = operation.step_after(step)?;
            let already_desired = proof.states().iter().all(|state| after.contains(state));
            if !already_desired {
                let provisional = writes
                    .iter()
                    .map(|key| {
                        let record = &self.resources[key];
                        (
                            key.clone(),
                            (record.possible.clone(), Arc::clone(&record.current)),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                for state in after {
                    let record = self.resources.get_mut(&state.key).unwrap();
                    if !record.possible.contains(&state.value) {
                        record.possible.push(state.value.clone());
                    }
                    record.current = Arc::clone(source);
                }
                let intent = WriteIntent {
                    operation,
                    step,
                    ownership: self.snapshot(),
                };
                if let Err(error) = control.before_write(&intent) {
                    self.undo_provisional(&provisional);
                    return Err(CoordinatorError::Gate(error));
                }
                self.durable_checkpoint = Some(self.checkpoint());
                if !operation.is_restoration() && control.cancellation_requested() {
                    self.undo_provisional(&provisional);
                    return Err(CoordinatorError::Gate(GateError::Cancelled));
                }
                let previously_possible = *possible_write;
                let previously_started = self.command_native_started;
                *possible_write = true;
                self.command_native_started = true;
                match operation.execute_step_expected(step, &proof) {
                    Err(failure) => {
                        if !failure.native_started {
                            self.undo_provisional(&provisional);
                            *possible_write = previously_possible;
                            self.command_native_started = previously_started;
                        }
                        self.capture_possible(source, &writes);
                        return Err(failure.error.into());
                    }
                    Ok(effect) if effect != operation.effect() => {
                        if operation.is_restoration() {
                            self.note_restore_effect(effect);
                        }
                        self.capture_possible(source, &writes);
                        return Err(CoordinatorError::EffectMismatch {
                            declared: operation.effect(),
                            actual: effect,
                        });
                    }
                    Ok(_) => {}
                }
                if operation.is_restoration() {
                    self.note_restore_effect(operation.effect());
                }
            }
            let states = match operation.verify_step_evidence(step) {
                Ok((Verification::Verified(effect), states))
                    if effect == operation.effect()
                        && states.iter().all(|state| after.contains(state)) =>
                {
                    states
                }
                Ok(_) => {
                    self.capture_possible(source, &writes);
                    return Err(CoordinatorError::Verification);
                }
                Err(error) => {
                    self.capture_possible(source, &writes);
                    return Err(error.into());
                }
            };
            for state in states {
                let record = self.resources.get_mut(&state.key).unwrap();
                record.confirmed = state.value;
                record.possible.clear();
                record.current = Arc::clone(source);
            }
            if !already_desired {
                let intent = WriteIntent {
                    operation,
                    step,
                    ownership: self.snapshot(),
                };
                control
                    .after_verified_write(&intent)
                    .map_err(CoordinatorError::Gate)?;
                self.durable_checkpoint = Some(self.checkpoint());
            }
            control.publish(self.snapshot());
        }
        let keys = operation
            .resources()
            .iter()
            .map(|claim| claim.key.clone())
            .collect::<Vec<_>>();
        let final_states = operation.observe_resources(&keys)?;
        if final_states.iter().any(|state| {
            !operation
                .resources()
                .iter()
                .any(|claim| claim.key == state.key && claim.desired == state.value)
        }) {
            return Err(CoordinatorError::ExternalDrift(keys));
        }
        if !operation.is_restoration() && control.cancellation_requested() {
            return Err(CoordinatorError::Gate(GateError::Cancelled));
        }
        Ok(())
    }
    fn undo_provisional(
        &mut self,
        before: &BTreeMap<ResourceKey, (Vec<DesiredStateToken>, Arc<Source>)>,
    ) {
        for (key, (possible, current)) in before {
            let record = self.resources.get_mut(key).unwrap();
            record.possible = possible.clone();
            record.current = Arc::clone(current);
        }
    }
    fn capture_possible(&mut self, source: &Arc<Source>, keys: &[ResourceKey]) {
        if let Ok(states) = source.operation.observe_resources(keys) {
            for state in states {
                let record = self.resources.get_mut(&state.key).unwrap();
                // Snapshots describe all planned/historical states. Only the
                // ledger's exact attempted instruction can authorize a value.
                let known =
                    state.value == record.confirmed || record.possible.contains(&state.value);
                if !known {
                    record.status = ModuleStatus::ExternalChange;
                }
            }
        }
    }
    fn mark_evidence_failure(&mut self, keys: &[ResourceKey], error: &CoordinatorError) {
        let affected = match error {
            CoordinatorError::ExternalDrift(drift) => drift.as_slice(),
            CoordinatorError::Catalog(CatalogError::Module(error))
                if matches!(error.kind, ErrorKind::ExternalChange | ErrorKind::Denied) =>
            {
                keys
            }
            _ => return,
        };
        let owners = affected
            .iter()
            .filter_map(|key| self.resources.get(key))
            .flat_map(|record| record.owners.iter().copied())
            .collect::<BTreeSet<_>>();
        for owner in owners {
            self.mark_pending(owner, affected, error);
        }
    }
    fn compensate_previous(
        &mut self,
        previous: &OwnerRecord,
        control: &mut dyn ExecutionControl,
    ) -> Result<(), CoordinatorError> {
        let proof = self.proof(&previous.source, &previous.resources, false)?;
        let config = previous.source.operation.config_envelope()?;
        let cleanup = Source::new(
            previous
                .source
                .operation
                .prepare_update_expected(&config, &proof)?,
        )?;
        let mut possible = false;
        self.execute(&cleanup, control, &mut possible)
    }
    fn release(&mut self, owner: OwnerId, control: &mut dyn ExecutionControl) -> CommandResult {
        let Some(request) = self.owners.get(&owner).cloned() else {
            return self.result(Outcome::Unchanged, None, control);
        };
        let mut pending = Vec::new();
        for key in &request.resources {
            if let Some(record) = self.resources.get_mut(key) {
                record.owners.remove(&owner);
                if record.owners.is_empty() {
                    pending.push(key.clone());
                    record.status = ModuleStatus::Restoring;
                }
            }
        }
        if pending.is_empty() {
            self.owners.remove(&owner);
            return self.result(Outcome::Released, None, control);
        }
        self.owners.get_mut(&owner).unwrap().resources = pending.clone();
        self.owners.get_mut(&owner).unwrap().status = ModuleStatus::Restoring;
        control.publish(self.snapshot());
        let mut error = None;
        // Per-field planning lets safe independent fields complete even if another
        // released field drifted. Native adapters still merge unrelated fields.
        for key in &pending {
            let Some(record) = self.resources.get(key) else {
                continue;
            };
            let original = Arc::clone(&record.original);
            let result = self.restore_one(&original, key, control);
            if let Err(failure) = result {
                self.mark_pending(owner, std::slice::from_ref(key), &failure);
                error = Some(failure);
            }
            // The next field's durable intent must not reference a field that
            // was already restored and removed from the resource ledger.
            if let Some(request) = self.owners.get_mut(&owner) {
                request
                    .resources
                    .retain(|key| self.resources.contains_key(key));
            }
        }
        let remaining = pending
            .into_iter()
            .filter(|key| self.resources.contains_key(key))
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            self.owners.remove(&owner);
            self.result(Outcome::Released, error, control)
        } else {
            let request = self.owners.get_mut(&owner).unwrap();
            request.resources = remaining;
            request.status = ModuleStatus::RecoveryRequired;
            self.result(Outcome::RecoveryRequired, error, control)
        }
    }
    fn restore_one(
        &mut self,
        original: &Arc<Source>,
        key: &ResourceKey,
        control: &mut dyn ExecutionControl,
    ) -> Result<(), CoordinatorError> {
        let proof = self.proof(original, std::slice::from_ref(key), true)?;
        let baseline = self.resources[key].baseline.clone();
        if proof.states()[0].value == baseline {
            self.resources.remove(key);
            return Ok(());
        }
        let prepared = original
            .operation
            .prepare_restore_expected(std::slice::from_ref(key), &proof)?;
        if prepared.resources()[0].desired != baseline {
            return Err(CoordinatorError::Catalog(CatalogError::InvalidPlan(
                "restorer changed canonical first baseline",
            )));
        }
        let source = Source::new(prepared)?;
        let mut possible = false;
        self.execute(&source, control, &mut possible)?;
        self.resources.remove(key);
        Ok(())
    }
    fn mark_pending(&mut self, owner: OwnerId, keys: &[ResourceKey], error: &CoordinatorError) {
        let status = match error {
            CoordinatorError::ExternalDrift(_) => ModuleStatus::ExternalChange,
            CoordinatorError::Catalog(CatalogError::Module(error))
                if error.kind == ErrorKind::Denied =>
            {
                ModuleStatus::PermissionDenied
            }
            CoordinatorError::Catalog(CatalogError::Module(error))
                if error.kind == ErrorKind::ExternalChange =>
            {
                ModuleStatus::ExternalChange
            }
            _ => ModuleStatus::RecoveryRequired,
        };
        for key in keys {
            if let Some(record) = self.resources.get_mut(key) {
                record.status = status;
            }
        }
        if let Some(request) = self.owners.get_mut(&owner) {
            request.status = ModuleStatus::RecoveryRequired;
        }
    }
    fn note_restore_effect(&mut self, effect: Effect) {
        if self
            .restoration_effect
            .is_none_or(|existing| effect_rank(effect) > effect_rank(existing))
        {
            self.restoration_effect = Some(effect);
        }
    }
    fn retire(
        &mut self,
        ended: &TargetIdentity,
        control: &mut dyn ExecutionControl,
    ) -> CommandResult {
        if !matches!(
            ended,
            TargetIdentity::Process(_) | TargetIdentity::Session(_)
        ) {
            return self.result(
                Outcome::Rejected,
                Some(CoordinatorError::ResourceSetChanged),
                control,
            );
        }
        let retired = self
            .resources
            .keys()
            .filter(|key| ended_lifetime(&key.target, ended))
            .cloned()
            .collect::<Vec<_>>();
        for key in &retired {
            self.resources.remove(key);
        }
        let owners = self
            .owners
            .iter()
            .filter(|(_, request)| {
                request.mode != ActivationMode::Manual
                    && ended_lifetime(request.source.operation.target(), ended)
            })
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        for request in self.owners.values_mut() {
            request.resources.retain(|key| !retired.contains(key));
        }
        let mut error = None;
        for owner in owners {
            let result = self.release(owner, control);
            if result.error.is_some() {
                error = result.error;
            }
        }
        self.owners
            .retain(|_, request| !request.resources.is_empty());
        if error.is_some() {
            self.result(Outcome::RecoveryRequired, error, control)
        } else {
            self.result(Outcome::Released, None, control)
        }
    }
}
fn active_status(effect: Effect) -> ModuleStatus {
    if effect == Effect::Immediate {
        ModuleStatus::VerifiedActive(effect)
    } else {
        ModuleStatus::StoredRestartRequired
    }
}
fn unresolved(status: ModuleStatus) -> bool {
    matches!(
        status,
        ModuleStatus::PartialFailure
            | ModuleStatus::ExternalChange
            | ModuleStatus::RecoveryRequired
            | ModuleStatus::PermissionDenied
    )
}
pub(super) fn effect_rank(effect: Effect) -> u8 {
    match effect {
        Effect::Immediate => 0,
        Effect::GameRestartRequired => 1,
        Effect::RestartRequired => 2,
    }
}
fn ended_lifetime(target: &TargetIdentity, ended: &TargetIdentity) -> bool {
    match (target, ended) {
        (TargetIdentity::Process(process), TargetIdentity::Session(session)) => {
            process.session == *session
        }
        (TargetIdentity::Session(_), TargetIdentity::Session(_))
        | (TargetIdentity::Process(_), TargetIdentity::Process(_)) => target == ended,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::tuning::testing::scalar::{ScalarConfig, ScalarModule};
    use crate::app::features::tuning::testing::{
        user_setting::{NativeValue, UserConfig, UserModule},
        value_setting::{ValueConfig, ValueModule},
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[derive(Default)]
    struct Control;
    impl ExecutionControl for Control {
        fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
            Ok(())
        }
        fn cancellation_requested(&self) -> bool {
            false
        }
        fn before_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
    }
    #[derive(Default)]
    struct FaultControl {
        cancel: bool,
        cancel_after_write: bool,
        deny_cleanup: bool,
        before_hook: Option<Box<dyn FnMut()>>,
        after_error: bool,
    }
    impl ExecutionControl for FaultControl {
        fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
            Ok(())
        }
        fn cancellation_requested(&self) -> bool {
            self.cancel
        }
        fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
            if self.deny_cleanup && intent.operation.is_restoration() {
                return Err(GateError::Rejected("cleanup denied"));
            }
            if let Some(mut hook) = self.before_hook.take() {
                hook();
            }
            Ok(())
        }
        fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
            if self.cancel_after_write && !intent.operation.is_restoration() {
                self.cancel = true;
            }
            if self.after_error {
                self.after_error = false;
                return Err(GateError::Rejected("progress publication failed"));
            }
            Ok(())
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
    }
    fn target() -> TargetIdentity {
        TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        })
    }
    fn owner(value: u64) -> OwnerId {
        OwnerId::new(value).unwrap()
    }
    fn session() -> SessionIdentity {
        SessionIdentity {
            user: UserIdentity {
                machine: MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                },
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(17).unwrap(),
            boot: BootId::new([2; 16]).unwrap(),
        }
    }
    fn scalar_fixture(initial: i32) -> (Coordinator, ScalarModule) {
        let scalar = ScalarModule::new(initial);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        (Coordinator::new(catalog), scalar)
    }
    fn acquire_scalar(
        coordinator: &mut Coordinator,
        scalar: &ScalarModule,
        id: u64,
        value: i32,
        mode: ActivationMode,
    ) -> CommandResult {
        coordinator.handle(
            Command::Acquire {
                owner: owner(id),
                config: scalar.config_envelope(&ScalarConfig { value }).unwrap(),
                target: target(),
                mode,
            },
            &mut Control,
        )
    }
    fn user_fixture() -> (Coordinator, UserModule, ValueModule) {
        let user = UserModule::new(None, true);
        let value = ValueModule::new(user.clone());
        let mut catalog = Catalog::default();
        catalog.register(user.clone()).unwrap();
        catalog.register(value.clone()).unwrap();
        (Coordinator::new(catalog), user, value)
    }
    fn acquire_user(
        coordinator: &mut Coordinator,
        module: &UserModule,
        id: u64,
        value: i32,
        enabled: bool,
    ) -> CommandResult {
        coordinator.handle(
            Command::Acquire {
                owner: owner(id),
                config: module
                    .config_envelope(&UserConfig { enabled, value })
                    .unwrap(),
                target: TargetIdentity::Session(session()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        )
    }
    fn acquire_value(
        coordinator: &mut Coordinator,
        module: &ValueModule,
        id: u64,
        value: i32,
    ) -> CommandResult {
        coordinator.handle(
            Command::Acquire {
                owner: owner(id),
                config: module.config_envelope(&ValueConfig { value }).unwrap(),
                target: TargetIdentity::Session(session()),
                mode: ActivationMode::Manual,
            },
            &mut Control,
        )
    }
    #[test]
    fn first_acquisition_retains_the_original_baseline_and_verified_owner() {
        let scalar = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        let result = coordinator.handle(
            Command::Acquire {
                owner: OwnerId::new(1).unwrap(),
                config,
                target: target(),
                mode: ActivationMode::Session,
            },
            &mut Control,
        );
        assert_eq!(
            result.snapshot.resources.len(),
            1,
            "accepted full claims must enter the ownership ledger"
        );
        assert_eq!(result.outcome, Outcome::Applied(Effect::Immediate));
        assert_eq!(scalar.value(), 12);
        assert_ne!(
            result.snapshot.resources[0].baseline,
            result.snapshot.resources[0].desired
        );
    }
    #[test]
    fn equal_requests_share_repeats_are_idempotent_and_last_owner_restores() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        assert_eq!(
            acquire_scalar(&mut coordinator, &scalar, 2, 12, ActivationMode::Session).outcome,
            Outcome::Shared
        );
        assert_eq!(
            acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session).outcome,
            Outcome::Unchanged
        );
        assert_eq!(scalar.writes(), 1);
        assert_eq!(
            coordinator
                .handle(Command::Release { owner: owner(1) }, &mut Control)
                .outcome,
            Outcome::Released
        );
        assert_eq!(scalar.value(), 12);
        coordinator.handle(Command::Release { owner: owner(2) }, &mut Control);
        assert_eq!(scalar.value(), 7);
        assert!(coordinator.snapshot().resources.is_empty());
        assert_eq!(
            coordinator
                .handle(Command::Release { owner: owner(2) }, &mut Control)
                .outcome,
            Outcome::Unchanged
        );
    }
    #[test]
    fn conflicting_overlap_rejects_entire_new_resource_set_without_claim_or_write() {
        let (mut coordinator, user, value) = user_fixture();
        acquire_value(&mut coordinator, &value, 2, 9);
        let before = coordinator.snapshot();
        let writes = user.writes();
        let rejected = acquire_user(&mut coordinator, &user, 1, 10, false);
        assert!(matches!(
            rejected.error,
            Some(CoordinatorError::Conflict(_))
        ));
        assert_eq!(rejected.snapshot, before);
        assert_eq!(user.writes(), writes);
        assert!(user.enabled());
    }
    #[test]
    fn mixed_shared_and_new_claims_keep_each_first_origin() {
        let (mut coordinator, user, value) = user_fixture();
        acquire_value(&mut coordinator, &value, 2, 9);
        acquire_user(&mut coordinator, &user, 1, 9, false);
        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.resources.len(), 2);
        let field = snapshot
            .resources
            .iter()
            .find(|resource| resource.key.field.as_str() == "user_value")
            .unwrap();
        assert_eq!(field.baseline_source.as_str(), "test_value_setting");
        assert_eq!(field.owners.len(), 2);
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(user.value(), Some(9));
        assert!(user.enabled());
        coordinator.handle(Command::Release { owner: owner(2) }, &mut Control);
        assert_eq!(user.value(), None);
    }
    #[test]
    fn cross_adapter_sole_owner_edit_restores_a_baseline_after_b_writes() {
        let (mut coordinator, user, value) = user_fixture();
        acquire_user(&mut coordinator, &user, 1, 9, false);
        acquire_value(&mut coordinator, &value, 2, 9);
        let original = coordinator
            .snapshot()
            .resources
            .iter()
            .find(|resource| resource.key.field.as_str() == "user_value")
            .unwrap()
            .baseline
            .clone();
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert!(user.enabled());
        assert_eq!(user.value(), Some(9));
        assert_eq!(
            acquire_value(&mut coordinator, &value, 2, 13).outcome,
            Outcome::Applied(Effect::RestartRequired)
        );
        let after = coordinator.snapshot();
        assert_eq!(after.resources[0].baseline, original);
        assert_eq!(
            after.resources[0].baseline_source.as_str(),
            "test_user_setting"
        );
        coordinator.handle(Command::Release { owner: owner(2) }, &mut Control);
        assert_eq!(user.value(), None);
        assert!(coordinator.snapshot().resources.is_empty());
    }
    #[test]
    fn sole_owner_update_keeps_first_baseline_shared_update_conflicts() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let first = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        acquire_scalar(&mut coordinator, &scalar, 1, 13, ActivationMode::Session);
        assert_eq!(
            coordinator.snapshot().resources[0].baseline,
            first.snapshot.resources[0].baseline
        );
        acquire_scalar(&mut coordinator, &scalar, 2, 13, ActivationMode::Session);
        assert!(matches!(
            acquire_scalar(&mut coordinator, &scalar, 1, 14, ActivationMode::Session).error,
            Some(CoordinatorError::Conflict(_))
        ));
        assert_eq!(scalar.value(), 13);
        coordinator.handle(Command::Release { owner: owner(2) }, &mut Control);
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(scalar.value(), 7);
    }
    #[test]
    fn no_op_retains_real_baseline_and_mode_edits_change_shutdown_policy() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let first = acquire_scalar(&mut coordinator, &scalar, 1, 7, ActivationMode::Session);
        assert_eq!(
            first.snapshot.resources[0].baseline,
            first.snapshot.resources[0].desired
        );
        assert_eq!(scalar.writes(), 0);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Manual);
        coordinator.shutdown(&mut Control);
        assert_eq!(scalar.value(), 12);
        assert_eq!(coordinator.snapshot().owners.len(), 1);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        coordinator.shutdown(&mut Control);
        assert_eq!(scalar.value(), 7);
        assert!(coordinator.snapshot().resources.is_empty());
    }
    #[test]
    fn shutdown_releases_session_share_without_restoring_manual_owner() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Manual);
        acquire_scalar(&mut coordinator, &scalar, 2, 12, ActivationMode::Session);
        coordinator.shutdown(&mut Control);
        assert_eq!(scalar.value(), 12);
        assert_eq!(coordinator.snapshot().owners[0].owner, owner(1));
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(scalar.value(), 7);
    }
    #[test]
    fn second_step_failure_is_compensated_without_success_or_lost_absence() {
        let (mut coordinator, user, _) = user_fixture();
        user.fail_next_enabled_step();
        let result = acquire_user(&mut coordinator, &user, 1, 9, false);
        assert_eq!(result.outcome, Outcome::Compensated);
        assert!(result.error.is_some());
        assert_eq!(user.value(), None);
        assert!(user.enabled());
        assert!(result.snapshot.resources.is_empty());
    }
    #[test]
    fn denied_last_release_retains_origin_and_can_be_retried() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        scalar.set_capability(Capability::Denied("Context lost".into()));
        let denied = coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(denied.outcome, Outcome::RecoveryRequired);
        assert_eq!(denied.snapshot.resources.len(), 1);
        scalar.set_capability(Capability::Available);
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(scalar.value(), 7);
        assert!(coordinator.snapshot().resources.is_empty());
    }
    #[test]
    fn external_drift_blocks_restore_and_independent_field_remains_restorable() {
        let (mut coordinator, user, _) = user_fixture();
        acquire_user(&mut coordinator, &user, 1, 9, false);
        let drift = NativeValue {
            kind: 99,
            bytes: vec![31, 42],
        };
        user.external_value(Some(drift.clone()));
        let result = coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(result.outcome, Outcome::RecoveryRequired);
        assert_eq!(user.native_value(), Some(drift));
        assert!(user.enabled());
        assert_eq!(
            result.snapshot.resources.len(),
            1,
            "safe independent restoration retires only its own field"
        );
    }
    #[test]
    fn process_activation_end_restores_machine_resource_instead_of_retiring_it() {
        let mut descriptor = crate::app::features::tuning::testing::scalar::descriptor();
        descriptor.target_scopes = vec![TargetScope::Process];
        let scalar = ScalarModule::with_descriptor(descriptor);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let process = TargetIdentity::Process(
            ProcessIdentity::new(
                session(),
                42,
                123,
                ImagePath::new(r"C:\Games\game.exe").unwrap(),
            )
            .unwrap(),
        );
        let mut coordinator = Coordinator::new(catalog);
        coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: process.clone(),
                mode: ActivationMode::Session,
            },
            &mut Control,
        );
        assert_eq!(scalar.value(), 12);
        coordinator.handle(Command::Retire { target: process }, &mut Control);
        assert_eq!(
            scalar.value(),
            7,
            "ancestor Machine baseline is restored, not discarded with activation PID"
        );
        assert!(coordinator.snapshot().resources.is_empty());
    }
    #[test]
    fn blanket_persistent_target_retirement_is_rejected_without_losing_baseline() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        let before = coordinator.snapshot();
        let result = coordinator.handle(Command::Retire { target: target() }, &mut Control);
        assert_eq!(result.outcome, Outcome::Rejected);
        assert_eq!(result.snapshot, before);
        assert_eq!(scalar.value(), 12);
    }
    #[test]
    fn attested_exact_native_lifetime_retires_without_restoration_or_backend_reads() {
        let process = |context, pid, token| {
            TargetIdentity::Process(
                ProcessIdentity::new(
                    context,
                    pid,
                    token,
                    ImagePath::new(r"C:\Games\game.exe").unwrap(),
                )
                .unwrap(),
            )
        };
        let exact_session = session();
        let mut other_logon = exact_session.clone();
        other_logon.logon = LogonId::new(18).unwrap();
        let mut other_boot = exact_session.clone();
        other_boot.boot = BootId::new([3; 16]).unwrap();
        let exact_process = process(exact_session.clone(), 42, 123);
        let cases = [
            (
                TargetScope::Process,
                exact_process.clone(),
                process(exact_session.clone(), 42, 124),
                exact_process.clone(),
            ),
            (
                TargetScope::Process,
                exact_process.clone(),
                process(exact_session.clone(), 43, 123),
                exact_process.clone(),
            ),
            (
                TargetScope::Process,
                exact_process.clone(),
                process(other_logon.clone(), 42, 123),
                exact_process.clone(),
            ),
            (
                TargetScope::Session,
                TargetIdentity::Session(exact_session.clone()),
                TargetIdentity::Session(other_logon.clone()),
                TargetIdentity::Session(exact_session.clone()),
            ),
            (
                TargetScope::Session,
                TargetIdentity::Session(exact_session.clone()),
                TargetIdentity::Session(other_boot),
                TargetIdentity::Session(exact_session.clone()),
            ),
            (
                TargetScope::Process,
                exact_process,
                TargetIdentity::Session(other_logon),
                TargetIdentity::Session(exact_session),
            ),
        ];
        for (scope, activation, unrelated_end, exact_end) in cases {
            let mut descriptor = crate::app::features::tuning::testing::scalar::descriptor();
            descriptor.target_scopes = vec![scope];
            descriptor.resources[0].scope = scope;
            let scalar = ScalarModule::with_descriptor(descriptor);
            let mut catalog = Catalog::default();
            catalog.register(scalar.clone()).unwrap();
            let mut coordinator = Coordinator::new(catalog);
            let applied = coordinator.handle(
                Command::Acquire {
                    owner: owner(1),
                    config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: activation.clone(),
                    mode: ActivationMode::Session,
                },
                &mut Control,
            );
            assert_eq!(applied.outcome, Outcome::Applied(Effect::Immediate));
            assert_eq!(applied.snapshot.resources[0].key.target, activation);
            // This is an authoritative end attestation, not an inference from
            // denial. No native query/write is permissible for an ended target.
            scalar.set_capability(Capability::Denied("expired native handle".into()));
            let reads = scalar.reads();
            let writes = scalar.writes();
            let unrelated = coordinator.handle(
                Command::Retire {
                    target: unrelated_end,
                },
                &mut Control,
            );
            assert_eq!(
                unrelated.snapshot, applied.snapshot,
                "a different PID, creation token, logon or boot cannot retire this incarnation"
            );
            let retired = coordinator.handle(
                Command::Retire {
                    target: exact_end.clone(),
                },
                &mut Control,
            );
            assert_eq!(retired.outcome, Outcome::Released);
            assert!(retired.error.is_none());
            assert!(retired.snapshot.resources.is_empty());
            assert!(retired.snapshot.owners.is_empty());
            let repeated = coordinator.handle(Command::Retire { target: exact_end }, &mut Control);
            assert_eq!(repeated.snapshot, retired.snapshot);
            assert_eq!(scalar.reads(), reads);
            assert_eq!(scalar.writes(), writes);
            assert_eq!(scalar.value(), 12);
        }
    }
    #[test]
    fn attempted_write_evidence_survives_denied_postwrite_observation() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let one = Arc::new(AtomicBool::new(true));
        let native = scalar.clone();
        scalar.set_execute_hook(Arc::new(move || {
            if one.swap(false, Ordering::SeqCst) {
                native.set_capability(Capability::Denied("postwrite query denied".into()));
            }
        }));
        let result = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        assert_eq!(result.outcome, Outcome::RecoveryRequired);
        assert_eq!(scalar.value(), 12);
        assert!(
            result.snapshot.resources[0]
                .possible
                .contains(&result.snapshot.resources[0].desired),
            "prewrite candidate must survive unreadable postwrite state"
        );
        scalar.set_capability(Capability::Available);
        let recovered = coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(recovered.outcome, Outcome::Released);
        assert_eq!(scalar.value(), 7);
    }
    #[test]
    fn classification_and_canonical_tokens_use_the_same_native_observation() {
        let (mut coordinator, user, _) = user_fixture();
        user.fail_next_value_after_write();
        let native = user.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let unknown = NativeValue {
            kind: 99,
            bytes: vec![31, 42],
        };
        let drift = unknown.clone();
        user.set_observe_hook(Arc::new(move || {
            if native.writes() > 0 {
                let n = reads.fetch_add(1, Ordering::SeqCst);
                if n == 1 {
                    native.external_value(Some(
                        crate::app::features::tuning::testing::user_setting::value_native(9),
                    ));
                } else {
                    native.external_value(Some(drift.clone()));
                }
            }
        }));
        let result = acquire_user(&mut coordinator, &user, 1, 9, false);
        user.clear_observe_hook();
        let token =
            crate::app::features::tuning::testing::user_setting::value_token(&Some(unknown));
        assert!(
            result
                .snapshot
                .resources
                .iter()
                .all(|record| !record.possible.contains(&token)),
            "unknown first read cannot be authorized by a different known second read"
        );
        assert_eq!(
            user.writes(),
            1,
            "unrecognized external bytes cannot be overwritten during compensation"
        );
    }
    #[test]
    fn write_then_error_is_compensated_and_progress_failure_never_reports_success() {
        let (mut coordinator, user, _) = user_fixture();
        user.fail_next_value_after_write();
        let result = acquire_user(&mut coordinator, &user, 1, 9, false);
        assert_eq!(result.outcome, Outcome::Compensated);
        assert!(result.error.is_some());
        assert_eq!(user.value(), None);
        assert_eq!(user.writes(), 2);
        let (mut coordinator, scalar) = scalar_fixture(7);
        let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let result = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config,
                target: target(),
                mode: ActivationMode::Session,
            },
            &mut FaultControl {
                after_error: true,
                ..Default::default()
            },
        );
        assert_eq!(result.outcome, Outcome::Compensated);
        assert!(result.error.is_some());
        assert_eq!(scalar.value(), 7);
    }
    #[test]
    fn cancelled_sole_owner_edit_compensates_previous_desired_and_keeps_first_baseline() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let first = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        let config = scalar.config_envelope(&ScalarConfig { value: 13 }).unwrap();
        let result = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config,
                target: target(),
                mode: ActivationMode::Session,
            },
            &mut FaultControl {
                cancel_after_write: true,
                ..Default::default()
            },
        );
        assert_eq!(result.outcome, Outcome::Compensated);
        assert_eq!(scalar.value(), 12);
        assert_eq!(
            result.snapshot.resources[0].baseline,
            first.snapshot.resources[0].baseline
        );
        assert_eq!(result.snapshot.owners.len(), 1);
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(scalar.value(), 7);
    }
    #[test]
    fn stale_expected_current_proof_cannot_overwrite_external_change_after_gate() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let native = scalar.clone();
        let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let result = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config,
                target: target(),
                mode: ActivationMode::Session,
            },
            &mut FaultControl {
                before_hook: Some(Box::new(move || native.external_change(31))),
                ..Default::default()
            },
        );
        assert_eq!(result.outcome, Outcome::Rejected);
        assert_eq!(scalar.value(), 31);
        assert_eq!(scalar.writes(), 0);
        assert!(result.snapshot.resources.is_empty());
    }
    #[test]
    fn restart_required_restoration_effect_survives_retired_resource() {
        let (mut coordinator, user, _) = user_fixture();
        acquire_user(&mut coordinator, &user, 1, 9, false);
        let result = coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(result.outcome, Outcome::Released);
        assert_eq!(result.restoration_effect, Some(Effect::RestartRequired));
        assert!(result.snapshot.resources.is_empty());
    }
    #[test]
    fn future_unattempted_snapshot_state_never_becomes_written_evidence() {
        let (mut coordinator, scalar) = scalar_fixture(0);
        scalar.set_plan_via(vec![1]);
        let native = scalar.clone();
        let once = Arc::new(AtomicBool::new(true));
        scalar.set_execute_hook(Arc::new(move || {
            if once.swap(false, Ordering::SeqCst) {
                native.external_change(2);
            }
        }));
        let result = acquire_scalar(&mut coordinator, &scalar, 1, 2, ActivationMode::Session);
        assert_eq!(
            scalar.value(),
            2,
            "future planned value is external until its instruction is attempted"
        );
        assert_eq!(scalar.writes(), 0);
        assert_eq!(result.outcome, Outcome::RecoveryRequired);
        assert!(!result.snapshot.resources[0]
            .possible
            .contains(&result.snapshot.resources[0].desired));
    }
    #[test]
    fn denied_later_step_does_not_retain_its_provisional_after_token() {
        let (mut coordinator, scalar) = scalar_fixture(0);
        scalar.set_plan_via(vec![1]);
        struct DenyLater {
            native: ScalarModule,
        }
        impl ExecutionControl for DenyLater {
            fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
                Ok(())
            }
            fn cancellation_requested(&self) -> bool {
                false
            }
            fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
                if !intent.operation.is_restoration() && intent.step == 1 {
                    self.native.external_change(2);
                    Err(GateError::Rejected("second step denied"))
                } else {
                    Ok(())
                }
            }
            fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
                Ok(())
            }
            fn publish(&mut self, _: CoordinatorSnapshot) {}
        }
        let result = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config: scalar.config_envelope(&ScalarConfig { value: 2 }).unwrap(),
                target: target(),
                mode: ActivationMode::Session,
            },
            &mut DenyLater {
                native: scalar.clone(),
            },
        );
        assert_eq!(
            result.outcome,
            Outcome::RecoveryRequired,
            "a refused gate proves the second native instruction never began"
        );
        assert_eq!(scalar.value(), 2);
        assert_eq!(scalar.writes(), 1);
        assert!(!result.snapshot.resources[0]
            .possible
            .contains(&result.snapshot.resources[0].desired));
        let retry = coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(retry.outcome, Outcome::RecoveryRequired);
        assert_eq!(scalar.value(), 2);
        assert_eq!(scalar.writes(), 1);
    }
    #[test]
    fn stale_precheck_before_native_call_does_not_promote_desired_external_value() {
        let (mut coordinator, scalar) = scalar_fixture(0);
        scalar.set_plan_via(vec![1]);
        struct ChangeLater {
            native: ScalarModule,
        }
        impl ExecutionControl for ChangeLater {
            fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
                Ok(())
            }
            fn cancellation_requested(&self) -> bool {
                false
            }
            fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
                if !intent.operation.is_restoration() && intent.step == 1 {
                    self.native.external_change(2);
                }
                Ok(())
            }
            fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
                Ok(())
            }
            fn publish(&mut self, _: CoordinatorSnapshot) {}
        }
        let result = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config: scalar.config_envelope(&ScalarConfig { value: 2 }).unwrap(),
                target: target(),
                mode: ActivationMode::Session,
            },
            &mut ChangeLater {
                native: scalar.clone(),
            },
        );
        assert_eq!(
            result.outcome,
            Outcome::RecoveryRequired,
            "expected-current refusal proves no second native call began"
        );
        assert_eq!(scalar.value(), 2);
        assert_eq!(scalar.writes(), 1);
        assert!(!result.snapshot.resources[0]
            .possible
            .contains(&result.snapshot.resources[0].desired));
    }
    #[test]
    fn historical_adapter_value_cannot_authorize_later_cross_adapter_restore() {
        let (mut coordinator, user, value) = user_fixture();
        acquire_user(&mut coordinator, &user, 1, 9, false);
        acquire_value(&mut coordinator, &value, 2, 9);
        coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        acquire_value(&mut coordinator, &value, 2, 13);
        let writes = user.writes();
        let native = user.clone();
        let first = coordinator.handle(
            Command::Release { owner: owner(2) },
            &mut FaultControl {
                before_hook: Some(Box::new(move || {
                    native.external_value(Some(
                        crate::app::features::tuning::testing::user_setting::value_native(9),
                    ))
                })),
                ..Default::default()
            },
        );
        assert_eq!(first.outcome, Outcome::RecoveryRequired);
        assert_eq!(user.value(), Some(9));
        assert_eq!(user.writes(), writes);
        let second = coordinator.handle(Command::Release { owner: owner(2) }, &mut Control);
        assert_eq!(
            second.outcome,
            Outcome::RecoveryRequired,
            "historical adapter knowledge is not an attempted write in this ownership incarnation"
        );
        assert_eq!(user.value(), Some(9));
        assert_eq!(user.writes(), writes);
        let old = crate::app::features::tuning::testing::user_setting::value_token(&Some(
            crate::app::features::tuning::testing::user_setting::value_native(9),
        ));
        assert!(!second.snapshot.resources[0].possible.contains(&old));
    }
    #[test]
    fn ordered_intermediate_write_is_not_skipped_when_final_equals_baseline() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        scalar.set_plan_via(vec![8]);
        let result = acquire_scalar(&mut coordinator, &scalar, 1, 7, ActivationMode::Session);
        assert_eq!(result.outcome, Outcome::Applied(Effect::Immediate));
        assert_eq!(scalar.value(), 7);
        assert_eq!(
            scalar.writes(),
            2,
            "both declared instructions establish their own exact states"
        );
        let released = coordinator.handle(Command::Release { owner: owner(1) }, &mut Control);
        assert_eq!(released.outcome, Outcome::Released);
        assert_eq!(scalar.writes(), 2);
    }
    #[test]
    fn native_effect_mismatch_cannot_be_reported_as_applied() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        scalar.return_next_native_effect(Effect::RestartRequired);
        let result = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        assert_eq!(result.outcome, Outcome::Compensated);
        assert_eq!(scalar.value(), 7);
        assert!(matches!(
            result.error,
            Some(CoordinatorError::EffectMismatch {
                declared: Effect::Immediate,
                actual: Effect::RestartRequired
            })
        ));
    }
    #[test]
    fn baseline_capture_never_recaptures_a_later_external_observation() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let native = scalar.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        scalar.set_observe_hook(Arc::new(move || {
            if reads.fetch_add(1, Ordering::SeqCst) == 1 {
                native.external_change(99);
            }
        }));
        let result = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        scalar.clear_observe_hook();
        assert!(matches!(
            result.error,
            Some(CoordinatorError::ExternalDrift(_))
        ));
        assert_eq!(scalar.writes(), 0);
        assert_eq!(scalar.value(), 99);
        assert!(result.snapshot.resources.is_empty());
    }
    #[test]
    fn active_edit_race_to_original_baseline_is_visible_and_does_not_reapply() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let first = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        let native = scalar.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        scalar.set_observe_hook(Arc::new(move || {
            if reads.fetch_add(1, Ordering::SeqCst) >= 2 {
                native.external_change(7);
            }
        }));
        let result = acquire_scalar(&mut coordinator, &scalar, 1, 13, ActivationMode::Session);
        scalar.clear_observe_hook();
        assert!(matches!(
            result.error,
            Some(CoordinatorError::ExternalDrift(_))
        ));
        assert_eq!(scalar.value(), 7);
        assert_eq!(scalar.writes(), 1);
        assert_eq!(
            result.snapshot.resources[0].baseline,
            first.snapshot.resources[0].baseline
        );
        assert_eq!(
            result.snapshot.resources[0].status,
            ModuleStatus::ExternalChange,
            "prewrite drift must remain visible after staged ownership rollback"
        );
        assert_eq!(
            coordinator
                .handle(Command::Release { owner: owner(1) }, &mut Control)
                .outcome,
            Outcome::Released
        );
        assert_eq!(scalar.writes(), 1);
    }
    #[test]
    fn equal_desired_owner_renewal_updates_activation_source_without_new_baseline() {
        let mut descriptor = crate::app::features::tuning::testing::scalar::descriptor();
        descriptor.target_scopes.push(TargetScope::Process);
        let scalar = ScalarModule::with_descriptor(descriptor);
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        let mut coordinator = Coordinator::new(catalog);
        let process = |pid| {
            TargetIdentity::Process(
                ProcessIdentity::new(
                    session(),
                    pid,
                    pid as u64 + 100,
                    ImagePath::new(r"C:\Games\game.exe").unwrap(),
                )
                .unwrap(),
            )
        };
        let first_target = process(10);
        let second_target = process(11);
        let config = scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let first = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config: config.clone(),
                target: first_target.clone(),
                mode: ActivationMode::Session,
            },
            &mut Control,
        );
        let renewed = coordinator.handle(
            Command::Acquire {
                owner: owner(1),
                config,
                target: second_target.clone(),
                mode: ActivationMode::Session,
            },
            &mut Control,
        );
        assert_eq!(renewed.outcome, Outcome::Unchanged);
        assert_eq!(renewed.snapshot.owners[0].target, second_target);
        assert_eq!(
            renewed.snapshot.resources[0].baseline,
            first.snapshot.resources[0].baseline
        );
        assert_eq!(scalar.writes(), 1);
        let old_end = coordinator.handle(
            Command::Retire {
                target: first_target,
            },
            &mut Control,
        );
        assert_eq!(old_end.snapshot.owners.len(), 1);
        assert_eq!(scalar.value(), 12);
        let new_end = coordinator.handle(
            Command::Retire {
                target: second_target,
            },
            &mut Control,
        );
        assert!(new_end.snapshot.owners.is_empty());
        assert_eq!(scalar.value(), 7);
    }
    struct FailedCommit;
    impl ExecutionControl for FailedCommit {
        fn cancellation_requested(&self) -> bool {
            false
        }
        fn before_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
        fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
            Err(GateError::Rejected("durable transition refused"))
        }
    }
    #[test]
    fn failed_no_write_transition_never_discards_or_changes_durable_ownership() {
        for scenario in 0..4 {
            let (mut coordinator, scalar) = scalar_fixture(7);
            if scenario != 0 {
                acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
            }
            if scenario == 3 {
                acquire_scalar(&mut coordinator, &scalar, 2, 12, ActivationMode::Session);
            }
            let before = coordinator.snapshot();
            let writes = scalar.writes();
            let command = match scenario {
                0 => Command::Acquire {
                    owner: owner(1),
                    config: scalar.config_envelope(&ScalarConfig { value: 7 }).unwrap(),
                    target: target(),
                    mode: ActivationMode::Session,
                },
                1 => Command::Acquire {
                    owner: owner(2),
                    config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: target(),
                    mode: ActivationMode::Session,
                },
                2 => Command::Acquire {
                    owner: owner(1),
                    config: scalar.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                    target: target(),
                    mode: ActivationMode::Manual,
                },
                _ => Command::Release { owner: owner(2) },
            };
            let result = coordinator.handle(command, &mut FailedCommit);
            assert_eq!(
                result.outcome,
                Outcome::Rejected,
                "a zero-write ownership transition still requires durable publication"
            );
            assert_eq!(result.snapshot, before);
            assert_eq!(scalar.writes(), writes);
            assert!(result.error.is_some());
        }
    }
    #[test]
    fn successful_native_restore_followed_by_failed_final_commit_retains_recovery_evidence() {
        let (mut coordinator, scalar) = scalar_fixture(7);
        let original = acquire_scalar(&mut coordinator, &scalar, 1, 12, ActivationMode::Session);
        let result = coordinator.handle(Command::Release { owner: owner(1) }, &mut FailedCommit);
        assert_eq!(
            result.outcome,
            Outcome::RecoveryRequired,
            "a failed final journal transition cannot erase restored evidence"
        );
        assert_eq!(scalar.value(), 7);
        assert_eq!(result.snapshot.resources.len(), 1);
        assert_eq!(result.snapshot.owners.len(), 1);
        assert_eq!(
            result.snapshot.resources[0].baseline,
            original.snapshot.resources[0].baseline
        );
        let writes = scalar.writes();
        let blocked = acquire_scalar(&mut coordinator, &scalar, 2, 13, ActivationMode::Session);
        assert_eq!(blocked.outcome, Outcome::RecoveryRequired);
        assert_eq!(scalar.writes(), writes);
        let observed = coordinator.handle(
            Command::Observe {
                feature: FeatureId::new("test_scalar").unwrap(),
                target: target(),
            },
            &mut FailedCommit,
        );
        assert!(matches!(observed.outcome, Outcome::Observed(_)));
    }
}
