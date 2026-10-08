//! Application-facing bounded requests over the existing saved transaction authority.
use super::{
    config::*,
    execution_bridge::*,
    platform::{codec::ModuleEnvelope, contract::*, runner::RunnerSnapshot},
    service::{HeadlessTuning, SaveError},
};
use crate::app::{
    features::rules::RulesContext,
    models::{AppStateStorage, AppToRun, RunningApps},
    shared::ids::{GroupId, RuleId},
};
use std::{
    sync::{
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) enum TuningRequest {
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
    pub runner: Option<RunnerSnapshot>,
    pub observation: Arc<ExecutionObservation>,
    pub shutdown_complete: bool,
    pub shutdown_outcome: Option<super::platform::runner::ShutdownResult>,
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
    session: Arc<Mutex<Option<(Arc<ExecutionObservation>, SessionFacts)>>>,
}
pub(crate) struct TuningRuntime {
    shared: Arc<Shared>,
    _worker: Option<JoinHandle<()>>,
}

pub(super) enum SessionDecision {
    Activate {
        game: GameActivation,
        feature: FeatureId,
        target: TargetIdentity,
    },
    Membership {
        owner: OwnerId,
        game: GameActivation,
    },
    Withdraw(OwnerId),
}
#[derive(Clone)]
pub(crate) enum SessionFacts {
    Unavailable,
    #[cfg(test)]
    Fixture {
        session: SessionIdentity,
        processes: Vec<ProcessIdentity>,
        targets: Vec<TargetIdentity>,
        exited: Vec<ProcessIdentity>,
    },
}
struct ResolvedSession<'a> {
    session: &'a SessionIdentity,
    processes: &'a [ProcessIdentity],
    targets: &'a [TargetIdentity],
    exited: &'a [ProcessIdentity],
}
impl SessionFacts {
    fn resolved(&self) -> Option<ResolvedSession<'_>> {
        match self {
            Self::Unavailable => None,
            #[cfg(test)]
            Self::Fixture {
                session,
                processes,
                targets,
                exited,
            } => Some(ResolvedSession {
                session,
                processes,
                targets,
                exited,
            }),
        }
    }
}
pub(super) fn session_decision(
    state: &AppStateStorage,
    observation: &ExecutionObservation,
    facts: &SessionFacts,
    catalog: &super::platform::catalog::Catalog,
) -> Option<SessionDecision> {
    if !observation.matches_state(state) {
        return None;
    }
    // Complete process facts do not qualify a native boot/logon/Game producer.
    let ResolvedSession {
        session,
        processes,
        targets,
        exited,
    } = facts.resolved()?;
    if processes.len() > 256 || targets.len() > 64 || exited.len() > 256 {
        return None;
    }
    let ExecutionObservation::Complete {
        members,
        foreground,
        ..
    } = observation
    else {
        return None;
    };
    let TuningLoad::Known(config) = state.tuning.decode() else {
        return None;
    };
    for association in &config.associations {
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.id == association.profile)?;
        for preference in &profile.modules {
            if !preference.enabled
                || !matches!(
                    preference.mode,
                    ActivationMode::Session | ActivationMode::ForegroundSession
                )
                || catalog.validate_config(&preference.config).is_err()
            {
                continue;
            }
            if preference.mode == ActivationMode::ForegroundSession
                && association.eligibility != ActivationEligibility::Foreground
            {
                continue;
            }
            let matches = |intent: &SavedIntent| {
                intent.config.feature == preference.feature
                    && matches!(&intent.binding, ActivationBinding::Game(game) if game.rule().group() == &association.group && game.rule().rule() == &association.rule)
            };
            if config.withdrawals.iter().any(matches)
                || config
                    .transitions
                    .iter()
                    .flatten()
                    .any(|t| matches(&t.old) || matches(&t.new))
            {
                continue;
            }
            let active = config.activations.iter().find(|intent| matches(intent));
            let member = members
                .iter()
                .find(|m| m.group == association.group && m.rule == association.rule);
            let typed = member.and_then(|member| {
                member
                    .instances
                    .iter()
                    .map(|facts| {
                        processes
                            .iter()
                            .find(|process| {
                                process.session == *session
                                    && process.pid() == facts.pid
                                    && process.creation_token() == facts.token
                                    && process.image_path.as_str() == facts.image.as_ref()
                            })
                            .cloned()
                    })
                    .collect::<Option<Vec<_>>>()
            });
            let Some(mut typed) = typed.filter(|typed| !typed.is_empty() && typed.len() <= 64)
            else {
                if let Some(active) = active {
                    let ActivationBinding::Game(game) = &active.binding else {
                        continue;
                    };
                    if game.processes().iter().all(|p| exited.contains(p)) {
                        return Some(SessionDecision::Withdraw(active.owner));
                    }
                }
                continue;
            };
            typed.sort();
            let focused = typed.iter().any(|p| {
                p.pid() == foreground.pid
                    && p.creation_token() == foreground.token
                    && p.image_path.as_str() == foreground.image.as_ref()
            });
            if association.eligibility == ActivationEligibility::Foreground && !focused {
                if let Some(active) = active {
                    return Some(SessionDecision::Withdraw(active.owner));
                }
                continue;
            }
            if let Some(active) = active {
                let ActivationBinding::Game(game) = &active.binding else {
                    continue;
                };
                if game.session() != session
                    || game.configured().profile != association.profile
                    || game.eligibility() != association.eligibility
                {
                    continue;
                }
                if typed != game.processes() {
                    if let Ok(next) = game.with_membership(typed) {
                        return Some(SessionDecision::Membership {
                            owner: active.owner,
                            game: next,
                        });
                    }
                }
            } else {
                let descriptor = catalog.descriptors().find(|d| d.id == preference.feature)?;
                let target = targets
                    .iter()
                    .find(|target| {
                        descriptor.target_scopes.contains(&target.scope())
                            && preference
                                .config
                                .target
                                .as_ref()
                                .is_none_or(|expected| expected == *target)
                    })?
                    .clone();
                let game = GameActivation::new(
                    ConfiguredIntent {
                        profile: association.profile,
                        generation: ConfigGeneration::new(
                            config.allocations.next_config_generation,
                        )
                        .ok()?,
                    },
                    RuleActivation::new(association.group.clone(), association.rule.clone())
                        .ok()?,
                    association.eligibility,
                    session.clone(),
                    typed,
                )
                .ok()?;
                return Some(SessionDecision::Activate {
                    game,
                    feature: preference.feature.clone(),
                    target,
                });
            }
        }
    }
    None
}
impl TuningRuntime {
    #[cfg(test)]
    pub(crate) fn observe_session_fixture(
        &self,
        observation: ExecutionObservation,
        facts: SessionFacts,
    ) {
        *self.shared.session.lock().unwrap() = Some((Arc::new(observation), facts));
    }
    #[cfg(test)]
    pub(crate) fn start_fixture(
        mut service: HeadlessTuning,
        running: Arc<tokio::sync::RwLock<RunningApps>>,
        state_path: std::path::PathBuf,
        journal_path: std::path::PathBuf,
        targets: Vec<TargetIdentity>,
    ) -> Self {
        service.configure_fixture(state_path, journal_path, targets);
        Self::start(service, running)
    }
    pub(crate) fn start(
        mut service: HeadlessTuning,
        running: Arc<tokio::sync::RwLock<RunningApps>>,
    ) -> Self {
        let initial = Arc::new(snapshot(
            &service,
            0,
            TuningOperationState::Idle,
            Arc::new(ExecutionObservation::Unknown(UnknownReason::Unavailable)),
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
            session: Default::default(),
        });
        let owned = shared.clone();
        let worker = thread::Builder::new()
            .name("tuning-session".into())
            .spawn(move || {
                let observed_state = Arc::new(RwLock::new(service.state().clone()));
                let mut bridge = ExecutionBridge::default();
                let mut operation = TuningOperationState::Idle;
                let mut shutting_down = false;
                let mut faulted = false;
                let mut drain_failed = false;
                if matches!(service.state().tuning.decode(), TuningLoad::Known(_)) {
                    let _ = service.start_saved_worker();
                }
                loop {
                    let closing = owned.admission.lock().unwrap().closing.is_some();
                    if closing {
                        use super::platform::runner::ShutdownResult;
                        if !shutting_down {
                            if let Ok(queued) = receiver.try_recv() {
                                operation = TuningOperationState::Refused {
                                    id: queued.id,
                                    error: SaveError::Pending,
                                };
                                owned.admission.lock().unwrap().pending = false;
                            }
                            if !faulted {
                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    service.drain_sessions()
                                })) {
                                    Ok(Ok(())) => {}
                                    Ok(Err(_)) => {
                                        drain_failed = true;
                                        operation = TuningOperationState::Recovering;
                                    }
                                    Err(_) => {
                                        faulted = true;
                                        operation = TuningOperationState::Recovering;
                                    }
                                }
                            }
                        }
                        let outcome = if shutting_down {
                            service.poll_shutdown()
                        } else {
                            shutting_down = true;
                            service.shutdown()
                        }
                        .unwrap_or(ShutdownResult::Complete);
                        let outcome = if faulted && outcome != ShutdownResult::TimedOut {
                            ShutdownResult::Faulted
                        } else {
                            outcome
                        };
                        let outcome = if drain_failed && outcome == ShutdownResult::Complete {
                            ShutdownResult::RecoveryRequired
                        } else {
                            outcome
                        };
                        let complete = outcome != ShutdownResult::TimedOut;
                        let mut next = snapshot(
                            &service,
                            service.saved_revision(),
                            operation.clone(),
                            bridge.latest(),
                            complete,
                        );
                        next.shutdown_outcome = Some(outcome);
                        *owned.latest.lock().unwrap() = Arc::new(next);
                        if complete {
                            break;
                        }
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(queued) => {
                            let result =
                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    if queued.revision != service.saved_revision() {
                                        Err(SaveError::Pending)
                                    } else {
                                        apply(&mut service, queued.request)
                                    }
                                })) {
                                    Ok(result) => result,
                                    Err(_) => {
                                        faulted = true;
                                        owned
                                            .admission
                                            .lock()
                                            .unwrap()
                                            .closing
                                            .get_or_insert_with(Instant::now);
                                        Err(SaveError::Recovering)
                                    }
                                };
                            operation = match result {
                                Ok(()) => TuningOperationState::Idle,
                                Err(SaveError::Recovering) => TuningOperationState::Recovering,
                                Err(error) => TuningOperationState::Refused {
                                    id: queued.id,
                                    error,
                                },
                            };
                            if let Ok(mut projection) = observed_state.write() {
                                *projection = service.state().clone();
                            }
                            bridge.request_refresh();
                            let mut next = snapshot(
                                &service,
                                service.saved_revision(),
                                operation.clone(),
                                bridge.latest(),
                                false,
                            );
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                observe_cards(&mut next, &service)
                            }))
                            .is_err()
                            {
                                faulted = true;
                                operation = TuningOperationState::Recovering;
                                next.operation = operation.clone();
                                owned
                                    .admission
                                    .lock()
                                    .unwrap()
                                    .closing
                                    .get_or_insert_with(Instant::now);
                            }
                            let mut admission = owned.admission.lock().unwrap();
                            admission.pending = false;
                            *owned.latest.lock().unwrap() = Arc::new(next);
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            owned.admission.lock().unwrap().closing = Some(Instant::now());
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    // Native inspection is outside both application locks and the GUI.
                    #[cfg(not(test))]
                    bridge.refresh(Instant::now(), &running, &observed_state);
                    #[cfg(test)]
                    let _ = &running;
                    let observation = bridge.latest();
                    let facts = SessionFacts::Unavailable;
                    #[cfg(test)]
                    let (observation, facts) = owned
                        .session
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or((observation, facts));
                    if let Some(decision) =
                        session_decision(service.state(), &observation, &facts, service.catalog())
                    {
                        let admitted = {
                            let mut admission = owned.admission.lock().unwrap();
                            if admission.closing.is_some() || admission.pending {
                                false
                            } else {
                                admission.pending = true;
                                true
                            }
                        };
                        if admitted {
                            let result =
                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    match decision {
                                        SessionDecision::Activate {
                                            game,
                                            feature,
                                            target,
                                        } => {
                                            service.activate_game(game, feature, target).map(|_| ())
                                        }
                                        SessionDecision::Membership { owner, game } => {
                                            service.update_game_membership(owner, game)
                                        }
                                        SessionDecision::Withdraw(owner) => {
                                            service.withdraw_owner(owner)
                                        }
                                    }
                                    .and_then(|()| service.reconcile())
                                    .and_then(|()| service.wait())
                                })) {
                                    Ok(result) => result,
                                    Err(_) => {
                                        faulted = true;
                                        owned
                                            .admission
                                            .lock()
                                            .unwrap()
                                            .closing
                                            .get_or_insert_with(Instant::now);
                                        Err(SaveError::Recovering)
                                    }
                                };
                            operation = match result {
                                Ok(()) => TuningOperationState::Idle,
                                Err(SaveError::Recovering) => TuningOperationState::Recovering,
                                Err(error) => TuningOperationState::Refused { id: 0, error },
                            };
                            if let Ok(mut projection) = observed_state.write() {
                                *projection = service.state().clone();
                            }
                            bridge.request_refresh();
                            let mut next = snapshot(
                                &service,
                                service.saved_revision(),
                                operation.clone(),
                                observation.clone(),
                                false,
                            );
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                observe_cards(&mut next, &service)
                            }))
                            .is_err()
                            {
                                faulted = true;
                                operation = TuningOperationState::Recovering;
                                next.operation = operation.clone();
                                owned
                                    .admission
                                    .lock()
                                    .unwrap()
                                    .closing
                                    .get_or_insert_with(Instant::now);
                            }
                            let mut admission = owned.admission.lock().unwrap();
                            admission.pending = false;
                            *owned.latest.lock().unwrap() = Arc::new(next);
                        }
                    }
                    let admission = owned.admission.lock().unwrap();
                    if admission.closing.is_none() && !admission.pending {
                        let latest = owned.latest.lock().unwrap().clone();
                        if !Arc::ptr_eq(&latest.observation, &observation) {
                            drop(admission);
                            let mut next = snapshot(
                                &service,
                                service.saved_revision(),
                                operation.clone(),
                                observation,
                                false,
                            );
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                observe_cards(&mut next, &service)
                            }))
                            .is_err()
                            {
                                faulted = true;
                                operation = TuningOperationState::Recovering;
                                next.operation = operation.clone();
                                owned
                                    .admission
                                    .lock()
                                    .unwrap()
                                    .closing
                                    .get_or_insert_with(Instant::now);
                            }
                            let admission = owned.admission.lock().unwrap();
                            if admission.closing.is_none() && !admission.pending {
                                *owned.latest.lock().unwrap() = Arc::new(next);
                            }
                        }
                    }
                }
            })
            .ok();
        Self {
            shared,
            _worker: worker,
        }
    }
    pub(crate) fn request(&self, request: TuningRequest) -> Result<u64, SaveError> {
        let mut admission = self.shared.admission.lock().unwrap();
        if admission.closing.is_some() || self._worker.is_none() {
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
        let mut next = (**self.shared.latest.lock().unwrap()).clone();
        next.operation = TuningOperationState::Pending { id };
        *self.shared.latest.lock().unwrap() = Arc::new(next);
        Ok(id)
    }
    pub(crate) fn snapshot(&self) -> Arc<TuningSnapshot> {
        let closing = self.shared.admission.lock().unwrap().closing;
        let latest = self.shared.latest.lock().unwrap().clone();
        if closing.is_some_and(|started| started.elapsed() >= Duration::from_secs(5))
            && !latest.shutdown_complete
            && latest.shutdown_outcome.is_none()
        {
            let mut timed_out = (*latest).clone();
            timed_out.shutdown_outcome = Some(super::platform::runner::ShutdownResult::TimedOut);
            Arc::new(timed_out)
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
    pub(crate) fn poll_interval(&self) -> Option<Duration> {
        let snapshot = self.snapshot();
        if snapshot.shutdown_complete {
            None
        } else if matches!(snapshot.operation, TuningOperationState::Pending { .. })
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
        TuningRequest::CreateProfile { name } => {
            service.create_profile(name, vec![])?;
        }
        TuningRequest::SetModulePreference {
            profile,
            preference,
            expected,
        } => service.set_module_preference(profile, *preference, expected)?,
        TuningRequest::DisableProfile { profile } => service.disable_profile(profile)?,
        TuningRequest::RemoveProfile { profile } => service.remove_profile(profile)?,
        TuningRequest::BindRule {
            group,
            rule,
            profile,
            eligibility,
        } => service.bind_rule(group, rule, profile, eligibility)?,
        TuningRequest::UnbindRule { group, rule } => service.unbind_rule(&group, &rule)?,
        TuningRequest::ActivateManual {
            profile,
            feature,
            target,
        } => {
            service.activate_manual(profile, feature, target)?;
        }
        TuningRequest::SaveBase { candidate, .. } => service.save_base(|saved| {
            *saved = *candidate;
            Ok(())
        })?,
        TuningRequest::RemoveRule { group, rule } => service.remove_rule(&group, &rule)?,
        TuningRequest::RemoveGroup { group } => service.remove_group(&group)?,
        TuningRequest::MoveRule {
            group,
            rule,
            target,
            index,
        } => service.move_rule_at(&group, &rule, &target, index)?,
        TuningRequest::EditRule { group, rule, app } => service.edit_rule(&group, &rule, *app)?,
    }
    if matches!(service.state().tuning.decode(), TuningLoad::Known(_)) {
        let _ = service.start_saved_worker();
    }
    if service.snapshot().is_some()
        && matches!(service.state().tuning.decode(), TuningLoad::Known(_))
    {
        service.reconcile()?;
        service.wait()?;
    }
    Ok(())
}
pub(super) fn observe_cards(snapshot: &mut TuningSnapshot, service: &HeadlessTuning) {
    let targets = service.available_targets();
    let saved = service.state().tuning.decode();
    let mut observed = std::collections::BTreeMap::new();
    for profile in &mut snapshot.profiles {
        for card in &mut profile.modules {
            let Some(descriptor) = snapshot.catalog.iter().find(|d| d.id == card.feature) else {
                continue;
            };
            if service.catalog().validate_config(&card.config).is_err() {
                card.capability =
                    Capability::Unsupported("Saved configuration codec unavailable".into());
                continue;
            }
            if let TuningLoad::Known(saved) = &saved {
                let belongs = |intent: &SavedIntent| {
                    intent.config.feature == card.feature
                        && intent
                            .binding
                            .configured()
                            .is_some_and(|binding| binding.profile == profile.id)
                };
                card.status = if saved.withdrawals.iter().any(belongs) {
                    Some(ModuleStatus::Restoring)
                } else if saved
                    .transitions
                    .iter()
                    .flatten()
                    .any(|transition| belongs(&transition.old) || belongs(&transition.new))
                {
                    Some(ModuleStatus::Applying)
                } else if card.enabled {
                    snapshot.runner.as_ref().and_then(|runner| {
                        runner
                            .coordinator
                            .owners
                            .iter()
                            .find(|owner| {
                                owner.source == card.feature
                                    && owner.mode == card.mode
                                    && owner.config.as_deref() == Some(&card.config)
                                    && saved.activations.iter().any(|intent| {
                                        belongs(intent)
                                            && intent.owner == owner.owner
                                            && intent.binding == owner.binding
                                            && intent.target == owner.target
                                            && intent.mode == owner.mode
                                            && owner.config.as_deref() == Some(&intent.config)
                                    })
                            })
                            .map(|owner| {
                                let projection =
                                    fresh_owner_observation(service, &runner.coordinator, owner);
                                let status = match &projection {
                                    Ok((status, _)) => *status,
                                    Err(super::platform::catalog::CatalogError::Module(error))
                                        if error.kind == ErrorKind::Denied =>
                                    {
                                        ModuleStatus::PermissionDenied
                                    }
                                    Err(super::platform::catalog::CatalogError::Module(error))
                                        if error.kind == ErrorKind::Unsupported =>
                                    {
                                        ModuleStatus::Unsupported
                                    }
                                    Err(_) => ModuleStatus::RecoveryRequired,
                                };
                                observed.insert(
                                    (owner.source.clone(), owner.target.clone()),
                                    projection.map(|(_, summary)| summary),
                                );
                                status
                            })
                    })
                } else {
                    None
                };
            }
            for target in &targets {
                if !descriptor.target_scopes.contains(&target.scope())
                    || card
                        .config
                        .target
                        .as_ref()
                        .is_some_and(|expected| expected != target)
                {
                    continue;
                }
                let result = observed
                    .entry((card.feature.clone(), target.clone()))
                    .or_insert_with(|| service.catalog().observe(&card.feature, target));
                match result {
                    Ok(summary) => {
                        card.capability = Capability::Available;
                        card.targets.push(target.clone());
                        card.observed = Some(summary.clone());
                    }
                    Err(super::platform::catalog::CatalogError::Module(error))
                        if error.kind == ErrorKind::Denied =>
                    {
                        card.capability = Capability::Denied("Observation denied".into());
                    }
                    Err(_) => {
                        card.capability = Capability::Unsupported("Observation unavailable".into());
                    }
                }
            }
        }
    }
}
fn fresh_owner_observation(
    service: &HeadlessTuning,
    retained: &super::platform::coordinator::CoordinatorSnapshot,
    owner: &super::platform::coordinator::OwnerView,
) -> Result<(ModuleStatus, ObservationSummary), super::platform::catalog::CatalogError> {
    let (summary, current) =
        service
            .catalog()
            .observe_with_resources(&owner.source, &owner.target, &owner.resources)?;
    for observed in current {
        let Some(expected) = retained.resources.iter().find(|resource| {
            resource.key == observed.key && resource.owners.contains(&owner.owner)
        }) else {
            return Ok((ModuleStatus::RecoveryRequired, summary));
        };
        if observed.value != expected.confirmed {
            return Ok((ModuleStatus::ExternalChange, summary));
        }
        if !expected.possible.is_empty() {
            return Ok((ModuleStatus::RecoveryRequired, summary));
        }
    }
    Ok((owner.status, summary))
}
pub(super) fn snapshot(
    service: &HeadlessTuning,
    revision: u64,
    operation: TuningOperationState,
    observation: Arc<ExecutionObservation>,
    shutdown_complete: bool,
) -> TuningSnapshot {
    let (configuration, profiles, associations) = match service.state().tuning.decode() {
        TuningLoad::Off => (TuningConfigurationState::Off, vec![], vec![]),
        TuningLoad::Unknown => (TuningConfigurationState::Unknown, vec![], vec![]),
        TuningLoad::Blocked => (TuningConfigurationState::Blocked, vec![], vec![]),
        TuningLoad::Known(config) => (
            TuningConfigurationState::Known,
            config
                .profiles
                .into_iter()
                .map(|p| ProfileCard {
                    id: p.id,
                    name: p.name,
                    modules: p
                        .modules
                        .into_iter()
                        .map(|m| ModuleCard {
                            title: service
                                .catalog()
                                .descriptors()
                                .find(|d| d.id == m.feature)
                                .map_or_else(|| "Unavailable module".into(), |d| d.title.into()),
                            feature: m.feature,
                            enabled: m.enabled,
                            mode: m.mode,
                            config: m.config,
                            status: None,
                            capability: Capability::Unsupported(
                                "Verified target unavailable".into(),
                            ),
                            targets: vec![],
                            observed: None,
                        })
                        .collect(),
                })
                .collect(),
            config.associations,
        ),
    };
    TuningSnapshot {
        revision,
        configuration,
        profiles,
        associations,
        catalog: service.catalog().descriptors().cloned().collect(),
        operation,
        runner: service.snapshot(),
        observation,
        shutdown_complete,
        shutdown_outcome: None,
        state: Arc::new(service.state().clone()),
        rules: service.rules().clone(),
    }
}
