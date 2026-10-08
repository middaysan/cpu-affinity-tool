//! One application worker saves candidates, reads tracked-process observations and dispatches modules.
use super::{
    config::*,
    execution_bridge::*,
    persistence::ActivationFacts,
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        controller::{LifetimeEvidence, OwnerKey, RecoveryIntent, ShutdownResult},
    },
    service::{HeadlessTuning, SaveError},
};
use crate::app::{
    features::rules::RulesContext,
    models::{AppStateStorage, AppToRun, RunningApps},
    shared::ids::{GroupId, RuleId},
};
use std::{
    collections::BTreeMap,
    sync::{
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub(crate) enum TuningRequest {
    ModuleAction {
        feature: FeatureId,
        action: super::presentation::ModuleAction,
    },
    CreateProfile {
        name: String,
    },
    SetModulePreference {
        profile: ProfileId,
        preference: Box<ModulePreference>,
        expected: Option<ModulePreference>,
    },
    DisableProfile {
        profile: ProfileId,
    },
    RemoveProfile {
        profile: ProfileId,
    },
    BindRule {
        group: GroupId,
        rule: RuleId,
        profile: ProfileId,
        eligibility: ActivationEligibility,
    },
    UnbindRule {
        group: GroupId,
        rule: RuleId,
    },
    ActivateManual {
        profile: ProfileId,
        feature: FeatureId,
        target: TargetIdentity,
    },
    SaveBase {
        candidate: Box<AppStateStorage>,
        expected_revision: u64,
    },
    RemoveRule {
        group: GroupId,
        rule: RuleId,
    },
    RemoveGroup {
        group: GroupId,
    },
    MoveRule {
        group: GroupId,
        rule: RuleId,
        target: GroupId,
        index: usize,
    },
    EditRule {
        group: GroupId,
        rule: RuleId,
        app: Box<AppToRun>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuningOperationState {
    Idle,
    Pending { id: u64 },
    Refused { id: u64, error: SaveError },
    Recovering,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuningConfigurationState {
    Off,
    Known,
    Unknown,
    Blocked,
}
#[derive(Clone, Debug)]
pub(crate) struct ModuleCard {
    pub feature: FeatureId,
    pub title: String,
    pub enabled: bool,
    pub mode: ActivationMode,
    pub config: ModuleEnvelope,
    pub status: Option<ModuleStatus>,
    pub capability: Capability,
    pub targets: Vec<TargetIdentity>,
    pub observed: Option<ObservationSummary>,
}
#[derive(Clone, Debug)]
pub(crate) struct ProfileCard {
    pub id: ProfileId,
    pub name: String,
    pub modules: Vec<ModuleCard>,
}
#[derive(Clone)]
pub(crate) struct TuningSnapshot {
    pub revision: u64,
    pub configuration: TuningConfigurationState,
    pub profiles: Vec<ProfileCard>,
    pub associations: Vec<Association>,
    pub catalog: Vec<ModuleDescriptor>,
    pub operation: TuningOperationState,
    pub readonly_cards: Vec<ModuleCard>,
    pub restoration_effect: Option<Effect>,
    pub operation_error: Option<ModuleError>,
    pub shutdown_complete: bool,
    pub shutdown_outcome: Option<ShutdownResult>,
    pub(crate) state: Arc<AppStateStorage>,
    pub(crate) rules: RulesContext,
}
struct Queued {
    id: u64,
    revision: u64,
    request: TuningRequest,
}
struct Admission {
    sender: SyncSender<Queued>,
    next: Option<u64>,
    pending: bool,
    closing: Option<Instant>,
}
struct Shared {
    admission: Mutex<Admission>,
    latest: Mutex<Arc<TuningSnapshot>>,
    #[cfg(test)]
    fixture: Mutex<Option<(Arc<ExecutionObservation>, Vec<(u32, u64)>)>>,
}
pub(crate) struct TuningRuntime {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl TuningRuntime {
    pub(crate) fn start(
        mut service: HeadlessTuning,
        running: Arc<tokio::sync::RwLock<RunningApps>>,
    ) -> Self {
        let observation = Arc::new(ExecutionObservation::Unknown(UnknownReason::Unavailable));
        let initial = Arc::new(snapshot(
            &service,
            TuningOperationState::Idle,
            observation,
            false,
            None,
            false,
        ));
        let (sender, receiver) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            admission: Mutex::new(Admission {
                sender,
                next: Some(1),
                pending: false,
                closing: None,
            }),
            latest: Mutex::new(initial),
            #[cfg(test)]
            fixture: Mutex::new(None),
        });
        let owned = shared.clone();
        let worker = thread::Builder::new()
            .name("tuning".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    #[cfg(windows)]
                    {
                        if !service.native_admitted {
                            match os_api::OS::tuning_legacy_evidence_exists() {
                                Ok(legacy) => service.legacy_recovery = legacy,
                                Err(error) => {
                                    service.legacy_recovery = true;
                                    service.native_failure(super::persistence::failure(
                                        "legacy recovery check",
                                        error,
                                    ));
                                }
                            }
                        }
                    }
                    service.start_selected_recovery();
                    let observed_state = Arc::new(RwLock::new(service.state().clone()));
                    let mut bridge = ExecutionBridge::default();
                    let mut operation = TuningOperationState::Idle;
                    let mut actors = retained_actors(&service);
                    let mut next_cards = Instant::now();
                    loop {
                        if owned.admission.lock().unwrap().closing.is_some() {
                            while let Ok(queued) = receiver.try_recv() {
                                operation = TuningOperationState::Refused {
                                    id: queued.id,
                                    error: SaveError::Pending,
                                };
                            }
                            let saved_verified =
                                !service.has_saved_source() || service.verify_saved().is_ok();
                            let may_restore = service.native_admitted
                                && !service.legacy_recovery
                                && saved_verified;
                            let mut failed = service.legacy_recovery || !saved_verified;
                            for owner in service
                                .controller
                                .owners()
                                .into_iter()
                                .filter(OwnerKey::is_session)
                            {
                                if !may_restore {
                                    failed = true;
                                    continue;
                                }
                                let exited = actors.get_mut(&owner).is_some_and(|actor| {
                                    actor.lifetime() == LifetimeEvidence::Exited
                                });
                                failed |= if exited {
                                    service
                                        .controller
                                        .release_if_exited(&owner, LifetimeEvidence::Exited)
                                        .is_err()
                                } else {
                                    service
                                        .controller
                                        .recover_owner(&owner, RecoveryIntent::Restore)
                                        .is_err()
                                };
                            }
                            service.controller.inspect();
                            let outcome = if failed
                                || !service.controller.can_admit()
                                || service.controller.owners().iter().any(OwnerKey::is_session)
                            {
                                ShutdownResult::RecoveryRequired
                            } else {
                                ShutdownResult::Complete
                            };
                            let completed = Arc::new(snapshot(
                                &service,
                                operation,
                                bridge.latest(),
                                true,
                                Some(outcome),
                                true,
                            ));
                            let mut admission = owned.admission.lock().unwrap();
                            *owned.latest.lock().unwrap() = completed;
                            admission.pending = false;
                            break;
                        }
                        match receiver.recv_timeout(Duration::from_millis(25)) {
                            Ok(queued) => {
                                let result = if queued.revision != service.saved_revision() {
                                    Err(SaveError::Invalid)
                                } else {
                                    apply(&mut service, queued.request)
                                };
                                operation = match result {
                                    Ok(()) => TuningOperationState::Idle,
                                    Err(error) => TuningOperationState::Refused {
                                        id: queued.id,
                                        error,
                                    },
                                };
                                *observed_state.write().unwrap() = service.state().clone();
                                bridge.request_refresh();
                                // Keep the pending latch until the completed snapshot has been published.
                                bridge.refresh(Instant::now(), &running, &observed_state);
                                let observation = worker_observation(&owned, &bridge, &mut actors);
                                reconcile_sessions(&mut service, observation.as_ref(), &mut actors);
                                service.controller.inspect();
                                next_cards = Instant::now() + Duration::from_secs(1);
                                let completed = Arc::new(snapshot(
                                    &service,
                                    operation.clone(),
                                    observation,
                                    false,
                                    None,
                                    true,
                                ));
                                let mut admission = owned.admission.lock().unwrap();
                                *owned.latest.lock().unwrap() = completed;
                                admission.pending = false;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                bridge.refresh(Instant::now(), &running, &observed_state);
                                let observation = worker_observation(&owned, &bridge, &mut actors);
                                reconcile_sessions(&mut service, observation.as_ref(), &mut actors);
                                let idle = {
                                    let admission = owned.admission.lock().unwrap();
                                    !admission.pending && admission.closing.is_none()
                                };
                                if idle && Instant::now() >= next_cards {
                                    service.controller.inspect();
                                    next_cards = Instant::now() + Duration::from_secs(1);
                                    let completed = Arc::new(snapshot(
                                        &service,
                                        operation.clone(),
                                        observation,
                                        false,
                                        None,
                                        true,
                                    ));
                                    let admission = owned.admission.lock().unwrap();
                                    if !admission.pending && admission.closing.is_none() {
                                        *owned.latest.lock().unwrap() = completed;
                                    }
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                }));
                if result.is_err() {
                    let mut latest = (**owned.latest.lock().unwrap()).clone();
                    latest.operation = TuningOperationState::Recovering;
                    latest.shutdown_outcome = Some(ShutdownResult::Faulted);
                    latest.shutdown_complete = true;
                    *owned.latest.lock().unwrap() = Arc::new(latest);
                    let mut admission = owned.admission.lock().unwrap();
                    admission.pending = false;
                    admission.closing.get_or_insert_with(Instant::now);
                }
            })
            .ok();
        Self { shared, worker }
    }
    pub(crate) fn request(&self, request: TuningRequest) -> Result<u64, SaveError> {
        let mut admission = self.shared.admission.lock().unwrap();
        if admission.closing.is_some() || self.worker.is_none() {
            return Err(SaveError::Recovering);
        }
        if admission.pending {
            return Err(SaveError::Pending);
        }
        let id = admission.next.ok_or(SaveError::Invalid)?;
        let revision = match &request {
            TuningRequest::SaveBase {
                expected_revision, ..
            } => *expected_revision,
            _ => self.shared.latest.lock().unwrap().revision,
        };
        match admission.sender.try_send(Queued {
            id,
            revision,
            request,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(SaveError::Pending),
            Err(TrySendError::Disconnected(_)) => return Err(SaveError::Recovering),
        }
        admission.next = id.checked_add(1);
        admission.pending = true;
        let mut latest = (**self.shared.latest.lock().unwrap()).clone();
        latest.operation = TuningOperationState::Pending { id };
        *self.shared.latest.lock().unwrap() = Arc::new(latest);
        Ok(id)
    }
    pub(crate) fn snapshot(&self) -> Arc<TuningSnapshot> {
        let closing = self.shared.admission.lock().unwrap().closing;
        let latest = self.shared.latest.lock().unwrap().clone();
        if closing.is_some_and(|t| t.elapsed() >= Duration::from_secs(5))
            && !latest.shutdown_complete
        {
            let mut timed = (*latest).clone();
            timed.shutdown_outcome = Some(ShutdownResult::TimedOut);
            Arc::new(timed)
        } else {
            latest
        }
    }
    pub(crate) fn begin_shutdown(&self) {
        self.shared
            .admission
            .lock()
            .unwrap()
            .closing
            .get_or_insert_with(Instant::now);
    }
    #[cfg(test)]
    pub(super) fn expire_shutdown_for_test(&self) {
        self.shared.admission.lock().unwrap().closing =
            Some(Instant::now() - Duration::from_secs(6));
    }
    #[cfg(test)]
    pub(super) fn observe_fixture(
        &self,
        observation: ExecutionObservation,
        exited: Vec<(u32, u64)>,
    ) {
        *self.shared.fixture.lock().unwrap() = Some((Arc::new(observation), exited));
    }
    pub(crate) fn poll_interval(&self) -> Option<Duration> {
        let s = self.snapshot();
        if s.shutdown_complete {
            None
        } else if matches!(s.operation, TuningOperationState::Pending { .. })
            || self.shared.admission.lock().unwrap().closing.is_some()
        {
            Some(Duration::from_millis(25))
        } else {
            Some(Duration::from_secs(1))
        }
    }
}
impl Drop for TuningRuntime {
    fn drop(&mut self) {
        self.begin_shutdown();
    }
}
fn apply(service: &mut HeadlessTuning, request: TuningRequest) -> Result<(), SaveError> {
    match request {
        TuningRequest::ModuleAction { feature, action } => service
            .registry()
            .get(&feature)
            .cloned()
            .ok_or(SaveError::Invalid)?
            .action(action)
            .map_err(|e| service.native_failure(e)),
        TuningRequest::CreateProfile { name } => service.create_profile(name, vec![]).map(|_| ()),
        TuningRequest::SetModulePreference {
            profile,
            preference,
            expected,
        } => service.set_module_preference(profile, *preference, expected),
        TuningRequest::DisableProfile { profile } => service.disable_profile(profile),
        TuningRequest::RemoveProfile { profile } => service.remove_profile(profile),
        TuningRequest::BindRule {
            group,
            rule,
            profile,
            eligibility,
        } => service.bind_rule(group, rule, profile, eligibility),
        TuningRequest::UnbindRule { group, rule } => service.unbind_rule(&group, &rule),
        TuningRequest::ActivateManual {
            profile,
            feature,
            target,
        } => service.activate_manual(profile, feature, target),
        TuningRequest::SaveBase { candidate, .. } => service.save_base(|saved| {
            *saved = *candidate;
            Ok(())
        }),
        TuningRequest::RemoveRule { group, rule } => service.remove_rule(&group, &rule),
        TuningRequest::RemoveGroup { group } => service.remove_group(&group),
        TuningRequest::MoveRule {
            group,
            rule,
            target,
            index,
        } => service.move_rule_at(&group, &rule, &target, index),
        TuningRequest::EditRule { group, rule, app } => service.edit_rule(&group, &rule, *app),
    }
}
struct Actor {
    target: TargetIdentity,
    config: ModuleEnvelope,
    facts: ProcessFacts,
    #[cfg(windows)]
    guard: Option<os_api::tuning::TuningProcessLifetimeGuard>,
    #[cfg(test)]
    fixture_lifetime: LifetimeEvidence,
}
fn retained_actors(service: &HeadlessTuning) -> BTreeMap<OwnerKey, Actor> {
    let mut actors = BTreeMap::new();
    for owner in service
        .controller
        .owners()
        .into_iter()
        .filter(OwnerKey::is_session)
    {
        let Some(record) = service.controller.owner_record(&owner) else {
            continue;
        };
        let Some(activation) = &record.activation else {
            continue;
        };
        let Some(target) = service.controller.target(&owner) else {
            continue;
        };
        #[cfg(windows)]
        let native = os_api::OS::tuning_process_identity(activation.pid)
            .ok()
            .filter(|native| {
                native.creation_time() == activation.token
                    && native.image_path() == activation.image
            })
            .map(Arc::new);
        #[cfg(windows)]
        let guard = native
            .as_ref()
            .and_then(|native| os_api::OS::tuning_open_process_lifetime(native).ok());
        let facts = ProcessFacts {
            pid: activation.pid,
            token: activation.token,
            image: Arc::from(activation.image.as_str()),
            #[cfg(windows)]
            native,
        };
        actors.insert(
            owner,
            Actor {
                target,
                config: record.config.clone(),
                facts,
                #[cfg(windows)]
                guard,
                #[cfg(test)]
                fixture_lifetime: LifetimeEvidence::Unknown,
            },
        );
    }
    actors
}
impl Actor {
    fn lifetime(&mut self) -> LifetimeEvidence {
        #[cfg(windows)]
        if let Some(guard) = &mut self.guard {
            return match guard.poll() {
                Ok(os_api::tuning::TuningProcessLifetime::VerifiedLive) => LifetimeEvidence::Live,
                Ok(os_api::tuning::TuningProcessLifetime::PositivelyExited) => {
                    LifetimeEvidence::Exited
                }
                _ => LifetimeEvidence::Unknown,
            };
        }
        #[cfg(test)]
        return self.fixture_lifetime;
        #[cfg(not(test))]
        LifetimeEvidence::Unknown
    }
}
fn worker_observation(
    shared: &Shared,
    bridge: &ExecutionBridge,
    actors: &mut BTreeMap<OwnerKey, Actor>,
) -> Arc<ExecutionObservation> {
    #[cfg(test)]
    if let Some((observation, exited)) = shared.fixture.lock().unwrap().as_ref() {
        for (owner, actor) in actors {
            actor.fixture_lifetime = if matches!(owner,OwnerKey::Session{pid,token,..} if exited.contains(&(*pid,*token)))
            {
                LifetimeEvidence::Exited
            } else {
                LifetimeEvidence::Unknown
            };
        }
        return observation.clone();
    }
    #[cfg(not(test))]
    let _ = (shared, actors);
    bridge.latest()
}
fn reconcile_sessions(
    service: &mut HeadlessTuning,
    observation: &ExecutionObservation,
    actors: &mut BTreeMap<OwnerKey, Actor>,
) {
    if !service.native_admitted || service.legacy_recovery {
        return;
    }
    let config = match service.state().tuning.decode() {
        TuningLoad::Known(config) => config,
        TuningLoad::Off if service.controller.owners().is_empty() => return,
        TuningLoad::Off => Box::new(TuningConfig::default()),
        _ => return,
    };
    if service.verify_saved().is_err() {
        return;
    }
    let mut desired = BTreeMap::new();
    let complete = observation.matches_state(service.state());
    if let ExecutionObservation::Complete {
        members,
        foreground,
        ..
    } = observation
    {
        if complete {
            for association in &config.associations {
                let Some(profile) = config.profiles.iter().find(|p| p.id == association.profile)
                else {
                    continue;
                };
                for member in members
                    .iter()
                    .filter(|m| m.group == association.group && m.rule == association.rule)
                {
                    for facts in &member.instances {
                        for preference in profile
                            .modules
                            .iter()
                            .filter(|m| m.enabled && m.mode != ActivationMode::Manual)
                        {
                            let foreground_only = preference.mode
                                == ActivationMode::ForegroundSession
                                || association.eligibility == ActivationEligibility::Foreground;
                            if foreground_only
                                && (foreground.pid != facts.pid
                                    || foreground.token != facts.token
                                    || foreground.image != facts.image)
                            {
                                continue;
                            }
                            let Some(module) = service.registry().get(&preference.feature) else {
                                continue;
                            };
                            let target = if module
                                .descriptor()
                                .target_scopes
                                .contains(&TargetScope::Process)
                            {
                                Some(TargetIdentity::Process {
                                    pid: facts.pid,
                                    creation_token: facts.token,
                                    image_path: facts.image.to_string(),
                                })
                            } else {
                                module.targets().into_iter().next()
                            };
                            let Some(target) = target else {
                                continue;
                            };
                            let owner = OwnerKey::Session {
                                profile: profile.id,
                                feature: preference.feature.clone(),
                                group: member.group.0.clone(),
                                rule: member.rule.0.clone(),
                                pid: facts.pid,
                                token: facts.token,
                            };
                            desired.insert(
                                owner,
                                (
                                    target,
                                    preference.config.clone(),
                                    facts.clone(),
                                    if foreground_only {
                                        ActivationEligibility::Foreground
                                    } else {
                                        ActivationEligibility::Running
                                    },
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    let existing: Vec<_> = actors.keys().cloned().collect();
    for owner in existing {
        let actor = actors.get_mut(&owner).unwrap();
        let preference = config
            .profiles
            .iter()
            .find(|p| p.id == owner.profile())
            .and_then(|p| {
                p.modules.iter().find(|m| {
                    m.enabled && m.mode != ActivationMode::Manual && &m.feature == owner.feature()
                })
            });
        let association=config.associations.iter().find(|a|a.profile==owner.profile() && matches!(&owner,OwnerKey::Session{group,rule,..} if &a.group.0==group && &a.rule.0==rule));
        let still_configured = preference.is_some() && association.is_some();
        let foreground_only = preference
            .is_some_and(|m| m.mode == ActivationMode::ForegroundSession)
            || association.is_some_and(|a| a.eligibility == ActivationEligibility::Foreground);
        let foreground_lost = complete
            && foreground_only
            && matches!(observation,ExecutionObservation::Complete{foreground,..} if foreground.pid!=actor.facts.pid || foreground.token!=actor.facts.token || foreground.image!=actor.facts.image);
        let release = !still_configured || foreground_lost;
        let lifetime = actor.lifetime();
        let released = if lifetime == LifetimeEvidence::Exited {
            service.controller.release_if_exited(&owner, lifetime)
        } else if release {
            service
                .controller
                .recover_owner(&owner, RecoveryIntent::Restore)
                .map(|_| true)
        } else {
            if service.controller.needs_recovery(&owner) {
                let present = complete
                    && matches!(observation,ExecutionObservation::Complete{members,..} if members.iter().any(|member|matches!(&owner,OwnerKey::Session{group,rule,..} if &member.group.0==group && &member.rule.0==rule) && member.instances.iter().any(|facts|facts.pid==actor.facts.pid && facts.token==actor.facts.token && facts.image==actor.facts.image)));
                let matching =
                    preference.is_some_and(|preference| preference.config == actor.config);
                let intent = if !matching {
                    RecoveryIntent::Restore
                } else if present
                    && (lifetime == LifetimeEvidence::Live || !service.has_saved_source())
                {
                    RecoveryIntent::Keep
                } else {
                    RecoveryIntent::Unknown
                };
                if let Err(error) = service.controller.recover_owner(&owner, intent) {
                    service.native_failure(error);
                }
                if intent == RecoveryIntent::Restore
                    && !service.controller.owners().contains(&owner)
                {
                    actors.remove(&owner);
                }
            }
            Ok(false)
        };
        if released == Ok(true) {
            actors.remove(&owner);
        }
    }
    if !complete {
        return;
    }
    for (owner, (target, config, facts, trigger)) in desired {
        if let Some(actor) = actors.get(&owner) {
            if actor.target == target
                && actor.config == config
                && service
                    .controller
                    .owner_record(&owner)
                    .and_then(|record| record.activation.as_ref())
                    .is_some_and(|activation| activation.trigger == trigger)
            {
                continue;
            }
        }
        if !service.controller.can_admit() {
            continue;
        }
        let Some(module) = service.registry().get(owner.feature()).cloned() else {
            continue;
        };
        #[cfg(windows)]
        let guard = if actors.contains_key(&owner) {
            None
        } else {
            match &facts.native {
                Some(native) => match os_api::OS::tuning_open_process_lifetime(native) {
                    Ok(guard) => Some(guard),
                    Err(_) => continue,
                },
                None => None,
            }
        };
        let activation = ActivationFacts {
            pid: facts.pid,
            token: facts.token,
            image: facts.image.to_string(),
            trigger,
        };
        let result = service.controller.acquire_session(
            owner.clone(),
            module,
            target.clone(),
            config.clone(),
            activation,
        );
        let acquired = result.is_ok();
        if let Err(error) = result {
            service.native_failure(error);
        }
        if acquired || service.controller.owners().contains(&owner) {
            if let Some(actor) = actors.get_mut(&owner) {
                actor.target = target;
                actor.config = config;
            } else {
                actors.insert(
                    owner,
                    Actor {
                        target,
                        config,
                        facts,
                        #[cfg(windows)]
                        guard,
                        #[cfg(test)]
                        fixture_lifetime: LifetimeEvidence::Unknown,
                    },
                );
            }
        }
    }
}
fn snapshot(
    service: &HeadlessTuning,
    operation: TuningOperationState,
    observation: Arc<ExecutionObservation>,
    shutdown_complete: bool,
    shutdown_outcome: Option<ShutdownResult>,
    observe: bool,
) -> TuningSnapshot {
    let load = service.state().tuning.decode();
    let mut profiles = Vec::new();
    let mut associations = Vec::new();
    let configuration = match &load {
        TuningLoad::Off => TuningConfigurationState::Off,
        TuningLoad::Known(_) => TuningConfigurationState::Known,
        TuningLoad::Unknown => TuningConfigurationState::Unknown,
        TuningLoad::Blocked => TuningConfigurationState::Blocked,
    };
    let mut templates = BTreeMap::new();
    let mut readonly_cards = Vec::new();
    for descriptor in service.registry().descriptors() {
        let module = service.registry().get(&descriptor.id).unwrap();
        let targets = if !observe || descriptor.target_scopes.contains(&TargetScope::Process) {
            vec![]
        } else {
            module
                .targets()
                .into_iter()
                .filter(TargetIdentity::valid)
                .take(256)
                .collect()
        };
        let observed =
            observe.then(|| module.observe(targets.first().unwrap_or(&TargetIdentity::Machine)));
        let capability = observed.as_ref().map_or_else(
            || Capability::Unsupported("No verified target".into()),
            |o| o.capability.clone(),
        );
        let card = ModuleCard {
            feature: descriptor.id.clone(),
            title: descriptor.title.into(),
            enabled: false,
            mode: ActivationMode::Manual,
            config: module.editor().default_config(),
            status: Some(ModuleStatus::Unmanaged),
            capability,
            targets,
            observed: observed.map(|o| ObservationSummary {
                summary: o.summary.summary,
                effect: o.effect,
            }),
        };
        if descriptor.access == ModuleAccess::ReadOnly {
            readonly_cards.push(card.clone());
        }
        templates.insert(descriptor.id, card);
    }
    if let TuningLoad::Known(config) = load {
        associations = config.associations;
        for profile in config.profiles {
            let mut modules = Vec::new();
            for preference in profile.modules {
                let mut card = templates
                    .get(&preference.feature)
                    .cloned()
                    .unwrap_or(ModuleCard {
                        feature: preference.feature.clone(),
                        title: preference.feature.as_str().into(),
                        enabled: false,
                        mode: preference.mode,
                        config: preference.config.clone(),
                        status: Some(ModuleStatus::Unsupported),
                        capability: Capability::Unsupported("Module unavailable".into()),
                        targets: vec![],
                        observed: None,
                    });
                card.enabled = preference.enabled;
                card.mode = preference.mode;
                card.config = preference.config;
                if let (Some(module), ExecutionObservation::Complete { members, .. }) = (
                    service.registry().get(&card.feature).filter(|module| {
                        observe
                            && observation.matches_state(service.state())
                            && module
                                .descriptor()
                                .target_scopes
                                .contains(&TargetScope::Process)
                    }),
                    observation.as_ref(),
                ) {
                    card.targets = members
                        .iter()
                        .filter(|member| {
                            associations.iter().any(|association| {
                                association.profile == profile.id
                                    && association.group == member.group
                                    && association.rule == member.rule
                            })
                        })
                        .flat_map(|member| &member.instances)
                        .take(256)
                        .map(|facts| TargetIdentity::Process {
                            pid: facts.pid,
                            creation_token: facts.token,
                            image_path: facts.image.to_string(),
                        })
                        .collect();
                    if let Some(target @ TargetIdentity::Process { pid, .. }) = card.targets.first()
                    {
                        let observed = module.observe(target);
                        card.capability = observed.capability;
                        card.observed = Some(ObservationSummary {
                            summary: format!("PID {pid}: {}", observed.summary.summary),
                            effect: observed.effect,
                        });
                    }
                }
                if service.registry().validate_config(&card.config).is_err() {
                    card.capability =
                        Capability::Unsupported("Saved configuration unavailable".into());
                }
                let owners = service.controller.owners();
                if card.enabled
                    || owners.iter().any(|owner| {
                        owner.profile() == profile.id && owner.feature() == &card.feature
                    })
                {
                    if let Some(status) = service.controller.status_for(profile.id, &card.feature) {
                        card.status = Some(status);
                    }
                }
                if service.legacy_recovery {
                    card.status = Some(ModuleStatus::RecoveryRequired);
                }
                modules.push(card);
            }
            profiles.push(ProfileCard {
                id: profile.id,
                name: profile.name,
                modules,
            });
        }
    }
    let operation = if service.legacy_recovery && matches!(operation, TuningOperationState::Idle) {
        TuningOperationState::Recovering
    } else {
        operation
    };
    TuningSnapshot {
        revision: service.saved_revision(),
        configuration,
        profiles,
        associations,
        catalog: service.registry().descriptors().collect(),
        operation,
        readonly_cards,
        restoration_effect: service.controller.restoration_effect,
        operation_error: service
            .operation_error
            .clone()
            .or_else(|| service.controller.error()),
        shutdown_complete,
        shutdown_outcome,
        state: Arc::new(service.state().clone()),
        rules: service.rules().clone(),
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;
    use crate::app::features::tuning::{
        platform::registry::{FieldChange, Module, ModuleObservation, Registry},
        presentation::ModuleEditor,
        simple_tests::Setting,
    };

    struct ProcessCard(Arc<Setting>);
    impl Module for ProcessCard {
        fn descriptor(&self) -> ModuleDescriptor {
            let mut descriptor = self.0.descriptor();
            descriptor.target_scopes = vec![TargetScope::Process];
            descriptor
        }
        fn editor(&self) -> &dyn ModuleEditor {
            self.0.editor()
        }
        fn targets(&self) -> Vec<TargetIdentity> {
            vec![]
        }
        fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
            ModuleObservation {
                capability: if matches!(target, TargetIdentity::Process { .. }) {
                    Capability::Available
                } else {
                    Capability::Unsupported("No process".into())
                },
                summary: ObservationSummary {
                    summary: format!("{target:?}"),
                    effect: Effect::Immediate,
                },
                effect: Effect::Immediate,
            }
        }
        fn fields(
            &self,
            target: &TargetIdentity,
            config: &ModuleEnvelope,
        ) -> Result<Vec<FieldChange>, ModuleError> {
            self.0.fields(target, config)
        }
        fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
            self.0.read(key)
        }
        fn write(
            &self,
            key: &ResourceKey,
            expected: &[u8],
            desired: &[u8],
        ) -> Result<(), ModuleError> {
            self.0.write(key, expected, desired)
        }
    }
    struct NoSave;
    impl super::super::StatePersistence for NoSave {
        fn save(
            &mut self,
            _: &mut AppStateStorage,
        ) -> Result<super::super::service::SavedState, SaveError> {
            Ok(super::super::service::SavedState::Unavailable)
        }
    }

    #[test]
    fn simple_ui_process_card_observes_only_its_profiles_associated_rule() {
        use crate::app::models::CoreGroup;
        let setting = Setting::new("process_card", false);
        let mut registry = Registry::default();
        registry
            .register(Arc::new(ProcessCard(setting.clone())))
            .unwrap();
        let apps = ["first", "second"].map(|name| {
            AppToRun::new_installed(
                name.into(),
                name.into(),
                os_api::PriorityClass::Normal,
                false,
            )
        });
        let state = AppStateStorage::test_fixture(|state| {
            state.groups = vec![CoreGroup {
                name: "Games".into(),
                cores: vec![],
                programs: apps.to_vec(),
                is_hidden: false,
                run_all_button: false,
            }];
            state.rule_identities = None;
        });
        let mut service =
            super::super::Tuning::from_registry(registry).headless(state, Box::new(NoSave));
        let group = service.rules().group_id_for_index(0).unwrap();
        let rules = [0, 1].map(|index| service.rules().rule_id_for_index(0, index).unwrap());
        let profile = service
            .create_profile(
                "Second game".into(),
                vec![ModulePreference {
                    feature: setting.descriptor().id,
                    enabled: true,
                    mode: ActivationMode::Session,
                    config: setting.config(12),
                }],
            )
            .unwrap();
        service
            .bind_rule(
                group.clone(),
                rules[1].clone(),
                profile,
                ActivationEligibility::Running,
            )
            .unwrap();
        let facts = [41, 42].map(|pid| ProcessFacts {
            pid,
            token: pid as u64 + 100,
            image: Arc::from(format!("game{pid}.exe")),
            #[cfg(windows)]
            native: None,
        });
        let observation = ExecutionObservation::fixture(
            service.state(),
            1,
            (0..2)
                .map(|index| Member {
                    key: apps[index].get_key(),
                    group: group.clone(),
                    rule: rules[index].clone(),
                    instances: vec![facts[index].clone()],
                })
                .collect(),
            facts[0].clone(),
        );
        let result = snapshot(
            &service,
            TuningOperationState::Idle,
            Arc::new(observation),
            false,
            None,
            true,
        );
        let card = &result.profiles[0].modules[0];
        assert_eq!(
            card.targets,
            vec![TargetIdentity::Process {
                pid: 42,
                creation_token: 142,
                image_path: "game42.exe".into(),
            }],
            "a profile must not show another game's targets"
        );
        assert!(card.observed.as_ref().unwrap().summary.contains("42"));
        let unknown = snapshot(
            &service,
            TuningOperationState::Idle,
            Arc::new(ExecutionObservation::Unknown(UnknownReason::Unavailable)),
            false,
            None,
            true,
        );
        assert!(unknown.profiles[0].modules[0].targets.is_empty());
    }
}
