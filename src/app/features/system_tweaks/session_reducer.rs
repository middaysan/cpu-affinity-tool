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
    ApplyTweak {
        feature: FeatureId,
        target: TargetIdentity,
    },
    UndoTweak {
        feature: FeatureId,
    },
    UndoAllTweaks,
    SelectTweakTarget {
        feature: FeatureId,
        target: TargetIdentity,
    },
    ConfirmDisplayTweak,
    RefreshPcChecks,
    ModuleAction {
        feature: FeatureId,
        action: super::presentation::ModuleAction,
    },
    // Legacy headless configuration commands remain supported for compatibility
    // and fixtures. The compact page offers previous-profile restoration only.
    #[cfg_attr(not(test), allow(dead_code))]
    CreateProfile {
        name: String,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    SetModulePreference {
        profile: ProfileId,
        preference: Box<ModulePreference>,
        expected: Option<ModulePreference>,
    },
    DisableProfile {
        profile: ProfileId,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    RemoveProfile {
        profile: ProfileId,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    BindRule {
        group: GroupId,
        rule: RuleId,
        profile: ProfileId,
        eligibility: ActivationEligibility,
    },
    #[allow(dead_code)]
    UnbindRule {
        group: GroupId,
        rule: RuleId,
    },
    #[cfg_attr(not(test), allow(dead_code))]
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
impl TuningRequest {
    fn is_base_change(&self) -> bool {
        matches!(
            self,
            Self::SaveBase { .. }
                | Self::RemoveRule { .. }
                | Self::RemoveGroup { .. }
                | Self::MoveRule { .. }
                | Self::EditRule { .. }
        )
    }
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
    pub tweak_cards: Vec<TweakCard>,
    pub pc_checks: Vec<super::pc_checks::PcCheck>,
    pub display_confirmation: Option<Instant>,
    pub has_tweak_changes: bool,
    pub revision: u64,
    pub configuration: TuningConfigurationState,
    pub profiles: Vec<ProfileCard>,
    pub catalog: Vec<ModuleDescriptor>,
    pub operation: TuningOperationState,
    #[cfg(test)]
    pub readonly_cards: Vec<ModuleCard>,
    pub restoration_effect: Option<Effect>,
    pub operation_error: Option<ModuleError>,
    pub shutdown_complete: bool,
    pub shutdown_outcome: Option<ShutdownResult>,
    pub(crate) state: Arc<AppStateStorage>,
    pub(crate) rules: RulesContext,
}
#[derive(Clone, Debug)]
pub(crate) struct TweakCard {
    pub feature: FeatureId,
    pub spec: super::presentation::TweakSpec,
    pub target: Option<TargetIdentity>,
    pub observed: Option<ObservationSummary>,
    pub status: Option<ModuleStatus>,
    pub ready: bool,
    pub owned: bool,
    pub baseline: Option<String>,
    pub capability: Capability,
    pub availability: super::platforms::TweakAvailability,
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
    modules_enabled: bool,
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
        service: HeadlessTuning,
        running: Arc<tokio::sync::RwLock<RunningApps>>,
    ) -> Self {
        Self::start_inner(
            service,
            running,
            #[cfg(test)]
            None,
        )
    }
    fn start_inner(
        mut service: HeadlessTuning,
        running: Arc<tokio::sync::RwLock<RunningApps>>,
        #[cfg(test)] startup_gate: Option<(SyncSender<()>, mpsc::Receiver<()>)>,
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
        let modules_enabled = service.modules_enabled;
        let shared = Arc::new(Shared {
            modules_enabled,
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
            .name(
                if modules_enabled {
                    "tuning"
                } else {
                    "state-save"
                }
                .into(),
            )
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    #[cfg(test)]
                    if let Some((entered, release)) = startup_gate {
                        entered.send(()).unwrap();
                        release.recv_timeout(Duration::from_secs(2)).unwrap();
                    }
                    #[cfg(windows)]
                    {
                        if modules_enabled && !service.native_admitted {
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
                    let mut actors = if modules_enabled {
                        retained_actors(&service)
                    } else {
                        BTreeMap::new()
                    };
                    let mut next_cards = Instant::now();
                    let mut next_display_retry = Instant::now();
                    loop {
                        // Reserve the existing latch before an autonomous save changes
                        // revision. Native work and snapshot construction run unlocked.
                        let expire_display = {
                            let mut admission = owned.admission.lock().unwrap();
                            if admission.closing.is_none()
                                && !admission.pending
                                && display_retry_due(
                                    Instant::now(),
                                    false,
                                    service.display_confirmation,
                                    &mut next_display_retry,
                                )
                            {
                                admission.pending = true;
                                true
                            } else {
                                false
                            }
                        };
                        if expire_display {
                            service.expire_display_tweak(false);
                            next_cards = Instant::now();
                            let completed = Arc::new(snapshot(
                                &service,
                                operation.clone(),
                                bridge.latest(),
                                false,
                                None,
                                true,
                            ));
                            let mut admission = owned.admission.lock().unwrap();
                            *owned.latest.lock().unwrap() = completed;
                            admission.pending = false;
                        }
                        if owned.admission.lock().unwrap().closing.is_some() {
                            while let Ok(queued) = receiver.try_recv() {
                                // Closing finishes already accepted classic saves, but never
                                // starts queued native actions or new session reconciliation.
                                let result = if !queued.request.is_base_change() {
                                    Err(SaveError::Pending)
                                } else if queued.revision != service.saved_revision() {
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
                            }
                            service.expire_display_tweak(true);
                            let outcome = if !modules_enabled {
                                ShutdownResult::Complete
                            } else {
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
                                if failed
                                    || service.display_confirmation.is_some()
                                    || !service.controller.can_admit()
                                    || service.controller.owners().iter().any(OwnerKey::is_session)
                                {
                                    ShutdownResult::RecoveryRequired
                                } else {
                                    ShutdownResult::Complete
                                }
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
                        match receiver.recv_timeout(Duration::from_millis(if modules_enabled {
                            25
                        } else {
                            250
                        })) {
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
                                let observation = if modules_enabled {
                                    *observed_state.write().unwrap() = service.state().clone();
                                    bridge.request_refresh();
                                    // Keep the pending latch until the completed snapshot has been published.
                                    bridge.refresh(Instant::now(), &running, &observed_state);
                                    let observation =
                                        worker_observation(&owned, &bridge, &mut actors);
                                    reconcile_sessions(
                                        &mut service,
                                        observation.as_ref(),
                                        &mut actors,
                                    );
                                    service.controller.inspect();
                                    observation
                                } else {
                                    Arc::new(ExecutionObservation::Unknown(
                                        UnknownReason::Unavailable,
                                    ))
                                };
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
                                if !modules_enabled {
                                    continue;
                                }
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
        if !self.shared.modules_enabled && !request.is_base_change() {
            return Err(SaveError::Invalid);
        }
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
        } else if self.shared.modules_enabled {
            Some(Duration::from_secs(1))
        } else {
            None
        }
    }
    pub(crate) fn modules_enabled(&self) -> bool {
        self.shared.modules_enabled
    }
}
impl Drop for TuningRuntime {
    fn drop(&mut self) {
        self.begin_shutdown();
    }
}
fn display_retry_due(
    now: Instant,
    closing: bool,
    confirmation: Option<Instant>,
    next_retry: &mut Instant,
) -> bool {
    if !confirmation.is_some_and(|deadline| closing || now >= deadline)
        || (!closing && now < *next_retry)
    {
        return false;
    }
    *next_retry = now + Duration::from_secs(1);
    true
}
fn apply(service: &mut HeadlessTuning, request: TuningRequest) -> Result<(), SaveError> {
    if !service.modules_enabled && !request.is_base_change() {
        return Err(SaveError::Invalid);
    }
    match request {
        TuningRequest::ApplyTweak { feature, target } => service.apply_tweak(feature, target),
        TuningRequest::UndoTweak { feature } => service.undo_tweak(&feature),
        TuningRequest::UndoAllTweaks => service.undo_all_tweaks(),
        TuningRequest::SelectTweakTarget { feature, target } => {
            service.select_tweak_target(feature, target)
        }
        TuningRequest::ConfirmDisplayTweak => service.confirm_display_tweak(),
        TuningRequest::RefreshPcChecks => {
            service.pc_checks = super::pc_checks::collect();
            Ok(())
        }
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
                            if !module.editor().availability().is_available() {
                                continue;
                            }
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
fn tweak_cards(
    service: &HeadlessTuning,
    templates: &BTreeMap<FeatureId, ModuleCard>,
    observe: bool,
) -> Vec<TweakCard> {
    let config = match service.state().tuning.decode() {
        TuningLoad::Known(config) => *config,
        _ => TuningConfig::default(),
    };
    let owners = service.controller.owners();
    let mut cards = Vec::new();
    for descriptor in service.registry().descriptors() {
        let module = service.registry().get(&descriptor.id).unwrap();
        let Some(spec) = module.editor().tweak() else {
            continue;
        };
        let availability = module.editor().availability();
        let target = service
            .tweak_targets
            .get(&descriptor.id)
            .cloned()
            .or_else(|| {
                // A saved disable can precede a failed restore. Recovery ownership,
                // including disabled internal profiles, outlives transient selection.
                owners.iter().rev().find_map(|owner| {
                    (owner.feature() == &descriptor.id
                        && config.profiles.iter().any(|profile| {
                            profile.id == owner.profile()
                                && super::service::is_tweak_profile(profile)
                        }))
                    .then(|| service.controller.target(owner))
                    .flatten()
                })
            })
            .or_else(|| {
                config
                    .profiles
                    .iter()
                    .rev()
                    .filter(|p| {
                        super::service::is_tweak_profile(p)
                            && p.modules[0].feature == descriptor.id
                            && p.modules[0].enabled
                    })
                    .find_map(|p| {
                        config
                            .manual_targets
                            .iter()
                            .find(|t| t.profile == p.id)
                            .map(|t| t.target.clone())
                    })
            })
            .or_else(|| {
                if module.editor().selects_saved_exe() {
                    None
                } else {
                    templates
                        .get(&descriptor.id)
                        .and_then(|c| c.targets.first().cloned())
                }
            });
        let observed = if observe && availability.is_available() {
            target.as_ref().map(|target| module.observe(target))
        } else {
            None
        };
        // Target discovery may fail while the diagnostic read still explains why.
        // Retain that explanation, without treating it as authority to write.
        let diagnostic = (observe
            && availability.is_available()
            && target.is_none()
            && !module.editor().selects_saved_exe())
        .then(|| templates.get(&descriptor.id))
        .flatten();
        let capability =
            if let super::platforms::TweakAvailability::Unavailable(reason) = availability {
                Capability::Unsupported(reason.into())
            } else {
                observed.as_ref().map_or_else(
                    || {
                        diagnostic.map_or_else(
                            || {
                                Capability::Unsupported(
                                    if module.editor().selects_saved_exe() {
                                        "Choose a saved EXE"
                                    } else if !observe {
                                        "Reading current setting…"
                                    } else {
                                        "Could not identify the setting target"
                                    }
                                    .into(),
                                )
                            },
                            |card| card.capability.clone(),
                        )
                    },
                    |o| o.capability.clone(),
                )
            };
        let suitable = if observe && matches!(capability, Capability::Available) {
            target
                .as_ref()
                .and_then(|target| module.editor().tweak_ready(target).ok())
        } else {
            None
        };
        let owner = owners.iter().find(|owner| {
            owner.feature() == &descriptor.id
                && config
                    .profiles
                    .iter()
                    .any(|p| p.id == owner.profile() && super::service::is_tweak_profile(p))
                && service.controller.target(owner).as_ref() == target.as_ref()
        });
        let owned = owner.is_some();
        let legacy_owner = owners.iter().any(|owner| {
            owner.feature() == &descriptor.id
                && !config
                    .profiles
                    .iter()
                    .any(|p| p.id == owner.profile() && super::service::is_tweak_profile(p))
        });
        let status = if let Some(owner) = owner {
            service.controller.status(owner)
        } else if legacy_owner {
            Some(ModuleStatus::RecoveryRequired)
        } else if suitable == Some(true) {
            Some(ModuleStatus::VerifiedActive(descriptor.effect))
        } else {
            Some(ModuleStatus::Unmanaged)
        };
        cards.push(TweakCard {
            availability,
            feature: descriptor.id.clone(),
            spec,
            target,
            observed: observed
                .map(|o| ObservationSummary {
                    summary: o.summary.summary,
                    effect: o.effect,
                })
                .or_else(|| diagnostic.and_then(|card| card.observed.clone())),
            status,
            ready: suitable == Some(false)
                && !owned
                && !legacy_owner
                && !service.legacy_recovery
                && service.controller.can_admit(),
            owned,
            baseline: owner.and_then(|owner| {
                module
                    .editor()
                    .format_baseline(&service.controller.baselines(owner))
            }),
            capability,
        });
    }
    cards.sort_by_key(|card| card.spec.order);
    cards
}

fn snapshot(
    service: &HeadlessTuning,
    operation: TuningOperationState,
    observation: Arc<ExecutionObservation>,
    shutdown_complete: bool,
    shutdown_outcome: Option<ShutdownResult>,
    observe: bool,
) -> TuningSnapshot {
    if !service.modules_enabled {
        return TuningSnapshot {
            tweak_cards: vec![],
            pc_checks: vec![],
            display_confirmation: None,
            has_tweak_changes: false,
            revision: service.saved_revision(),
            configuration: TuningConfigurationState::Off,
            profiles: vec![],
            catalog: vec![],
            operation,
            #[cfg(test)]
            readonly_cards: vec![],
            restoration_effect: None,
            operation_error: None,
            shutdown_complete,
            shutdown_outcome,
            state: Arc::new(service.state().clone()),
            rules: service.rules().clone(),
        };
    }
    let load = service.state().tuning.decode();
    let mut profiles = Vec::new();
    let configuration = match &load {
        TuningLoad::Off => TuningConfigurationState::Off,
        TuningLoad::Known(_) => TuningConfigurationState::Known,
        TuningLoad::Unknown => TuningConfigurationState::Unknown,
        TuningLoad::Blocked => TuningConfigurationState::Blocked,
    };
    let mut templates = BTreeMap::new();
    #[cfg(test)]
    let mut readonly_cards = Vec::new();
    for descriptor in service.registry().descriptors() {
        let module = service.registry().get(&descriptor.id).unwrap();
        // The compact page does not show legacy audio/PnP/USB inventories.
        // Their retained adapters/actions are compatible data, not idle work.
        let availability = module.editor().availability();
        let observe_module =
            observe && descriptor.access == ModuleAccess::Managed && availability.is_available();
        let targets = if !observe_module || descriptor.target_scopes.contains(&TargetScope::Process)
        {
            vec![]
        } else {
            module
                .targets()
                .into_iter()
                .filter(TargetIdentity::valid)
                .take(256)
                .collect()
        };
        let observed = observe_module
            .then(|| module.observe(targets.first().unwrap_or(&TargetIdentity::Machine)));
        let capability =
            if let super::platforms::TweakAvailability::Unavailable(reason) = availability {
                Capability::Unsupported(reason.into())
            } else {
                observed.as_ref().map_or_else(
                    || Capability::Unsupported("No verified target".into()),
                    |o| o.capability.clone(),
                )
            };
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
        #[cfg(test)]
        if descriptor.access == ModuleAccess::ReadOnly {
            readonly_cards.push(card.clone());
        }
        templates.insert(descriptor.id, card);
    }
    if let TuningLoad::Known(config) = load {
        let associations = config.associations;
        for profile in config.profiles {
            if super::service::is_tweak_profile(&profile) {
                continue;
            }
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
                        if module.editor().availability().is_available() {
                            let observed = module.observe(target);
                            card.capability = observed.capability;
                            card.observed = Some(ObservationSummary {
                                summary: format!("PID {pid}: {}", observed.summary.summary),
                                effect: observed.effect,
                            });
                        }
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
        tweak_cards: tweak_cards(service, &templates, observe),
        pc_checks: service.pc_checks.clone(),
        display_confirmation: service.display_confirmation,
        has_tweak_changes: service.controller.owners().iter().any(|owner| {
            if let TuningLoad::Known(config) = service.state().tuning.decode() {
                config.profiles.iter().any(|profile| {
                    profile.id == owner.profile() && super::service::is_tweak_profile(profile)
                })
            } else {
                false
            }
        }),
        revision: service.saved_revision(),
        configuration,
        profiles,
        catalog: service.registry().descriptors().collect(),
        operation,
        #[cfg(test)]
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
    use crate::app::features::system_tweaks::{
        platform::registry::{FieldChange, Module, ModuleObservation, Registry},
        presentation::ModuleEditor,
        simple_tests::Setting,
    };

    #[test]
    fn accepted_classic_save_is_persisted_before_shutdown_completes() {
        struct RecordingSave(Arc<Mutex<Vec<usize>>>);
        impl super::super::StatePersistence for RecordingSave {
            fn save(
                &mut self,
                candidate: &mut AppStateStorage,
            ) -> Result<super::super::service::SavedState, SaveError> {
                self.0.lock().unwrap().push(candidate.theme_index);
                Ok(super::super::service::SavedState::Unavailable)
            }
        }
        let saved = Arc::new(Mutex::new(Vec::new()));
        let state = AppStateStorage::test_fixture(|state| state.theme_index = 0);
        let service = super::super::Tuning::from_registry(Registry::default())
            .headless(state.clone(), Box::new(RecordingSave(saved.clone())))
            .without_modules();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let runtime = TuningRuntime::start_inner(
            service,
            Arc::new(tokio::sync::RwLock::new(RunningApps::default())),
            Some((entered_tx, release_rx)),
        );
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut candidate = state;
        candidate.theme_index = 1;
        let accepted = runtime.request(TuningRequest::SaveBase {
            candidate: Box::new(candidate.clone()),
            expected_revision: 0,
        });
        runtime.begin_shutdown();
        let late_request = runtime.request(TuningRequest::SaveBase {
            candidate: Box::new(candidate),
            expected_revision: 0,
        });
        release_tx.send(()).unwrap();
        assert!(accepted.is_ok(), "the save was accepted before close");
        assert_eq!(late_request, Err(SaveError::Recovering));
        let until = Instant::now() + Duration::from_secs(2);
        while !runtime.snapshot().shutdown_complete && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        let terminal = runtime.snapshot();
        assert!(terminal.shutdown_complete);
        assert_eq!(*saved.lock().unwrap(), vec![1]);
        assert_eq!(terminal.revision, 1);
        assert_eq!(terminal.state.theme_index, 1);
        assert_eq!(terminal.operation, TuningOperationState::Idle);
        assert_eq!(terminal.shutdown_outcome, Some(ShutdownResult::Complete));
    }

    #[test]
    fn compact_catalog_does_not_poll_hidden_readonly_inventory() {
        let mut registry = Registry::default();
        struct HiddenInventory(Arc<Setting>);
        impl Module for HiddenInventory {
            fn descriptor(&self) -> ModuleDescriptor {
                let mut descriptor = self.0.descriptor();
                descriptor.access = ModuleAccess::ReadOnly;
                descriptor
            }
            fn editor(&self) -> &dyn ModuleEditor {
                self.0.editor()
            }
            fn targets(&self) -> Vec<TargetIdentity> {
                panic!("Hidden inventory must not enumerate targets")
            }
            fn observe(&self, _: &TargetIdentity) -> ModuleObservation {
                panic!("Hidden inventory must not be polled")
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
        let module = Arc::new(HiddenInventory(Setting::new("hidden_inventory", false)));
        registry.register(module).unwrap();
        let service = super::super::Tuning::from_registry(registry).headless(
            crate::app::models::AppStateStorage::test_fixture(|_| {}),
            Box::new(super::super::service::SelectedStatePersistence),
        );
        let result = snapshot(
            &service,
            TuningOperationState::Idle,
            Arc::new(ExecutionObservation::Unknown(
                super::super::execution_bridge::UnknownReason::Unavailable,
            )),
            false,
            None,
            true,
        );
        assert_eq!(result.readonly_cards.len(), 1);
        assert!(result.readonly_cards[0].observed.is_none());
    }

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

    struct MovingDisplay(Arc<Setting>, Arc<Mutex<TargetIdentity>>);
    impl Module for MovingDisplay {
        fn descriptor(&self) -> ModuleDescriptor {
            let mut descriptor = self.0.descriptor();
            descriptor.target_scopes = vec![TargetScope::PnpDevice, TargetScope::Display];
            descriptor
        }
        fn editor(&self) -> &dyn ModuleEditor {
            self.0.editor()
        }
        fn targets(&self) -> Vec<TargetIdentity> {
            vec![self.1.lock().unwrap().clone()]
        }
        fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
            self.0.observe(target)
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
    fn display_selection_fixture() -> (HeadlessTuning, Arc<Setting>, Arc<Mutex<TargetIdentity>>) {
        let setting = Setting::new("display_refresh", false);
        let primary = Arc::new(Mutex::new(TargetIdentity::Device {
            identity: "monitor-a".into(),
        }));
        let mut registry = Registry::default();
        registry
            .register(Arc::new(MovingDisplay(setting.clone(), primary.clone())))
            .unwrap();
        let mut service = super::super::Tuning::from_registry(registry)
            .headless(AppStateStorage::test_fixture(|_| {}), Box::new(NoSave));
        service.admit_test_modules();
        (service, setting, primary)
    }
    fn display_selection_card(service: &HeadlessTuning) -> TweakCard {
        snapshot(
            service,
            TuningOperationState::Idle,
            Arc::new(ExecutionObservation::Unknown(UnknownReason::Unavailable)),
            false,
            None,
            true,
        )
        .tweak_cards
        .into_iter()
        .next()
        .unwrap()
    }
    #[test]
    fn accepted_classic_save_precedes_pending_display_expiry() {
        for closing in [true, false] {
            let (mut service, setting, primary) = display_selection_fixture();
            let original = primary.lock().unwrap().clone();
            service
                .apply_tweak(setting.descriptor().id, original)
                .unwrap();
            assert_eq!(setting.value(), vec![12]);
            service.display_confirmation = Some(Instant::now());
            let expected_revision = service.saved_revision();
            let mut candidate = service.state().clone();
            candidate.theme_index = 1;
            let (entered_tx, entered_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let runtime = TuningRuntime::start_inner(
                service,
                Arc::new(tokio::sync::RwLock::new(RunningApps::default())),
                Some((entered_tx, release_rx)),
            );
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let accepted = runtime.request(TuningRequest::SaveBase {
                candidate: Box::new(candidate),
                expected_revision,
            });
            if closing {
                runtime.begin_shutdown();
            }
            release_tx.send(()).unwrap();
            assert!(accepted.is_ok());
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                let snapshot = runtime.snapshot();
                if (closing && snapshot.shutdown_complete)
                    || (!closing && snapshot.display_confirmation.is_none())
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            let completed = runtime.snapshot();
            assert_eq!(completed.state.theme_index, 1, "closing={closing}");
            assert_eq!(completed.revision, expected_revision + 2);
            assert!(completed.display_confirmation.is_none());
            assert_eq!(setting.value(), vec![7]);
            assert_eq!(completed.operation, TuningOperationState::Idle);
            if closing {
                assert!(completed.shutdown_complete);
            }
        }
    }
    #[test]
    fn display_selection_follows_current_primary_after_individual_and_all_undo() {
        for all in [false, true] {
            let (mut service, setting, primary) = display_selection_fixture();
            let original = primary.lock().unwrap().clone();
            let feature = setting.descriptor().id;
            service
                .apply_tweak(feature.clone(), original.clone())
                .unwrap();
            service.confirm_display_tweak().unwrap();
            let current = TargetIdentity::Device {
                identity: "monitor-b".into(),
            };
            *primary.lock().unwrap() = current.clone();
            assert_eq!(display_selection_card(&service).target, Some(original));
            if all {
                service.undo_all_tweaks().unwrap();
            } else {
                service.undo_tweak(&feature).unwrap();
            }
            assert_eq!(setting.value(), vec![7]);
            let card = display_selection_card(&service);
            assert_eq!(card.target, Some(current.clone()));
            assert!(!card.owned);
            assert!(card.ready);
            service.apply_tweak(feature, card.target.unwrap()).unwrap();
            assert_eq!(display_selection_card(&service).target, Some(current));
        }
    }
    #[test]
    fn display_selection_noop_apply_does_not_pin_an_unowned_monitor() {
        let (mut service, setting, primary) = display_selection_fixture();
        setting
            .fields
            .lock()
            .unwrap()
            .insert(ResourceId::new("value").unwrap(), vec![12]);
        let original = primary.lock().unwrap().clone();
        service
            .apply_tweak(setting.descriptor().id, original)
            .unwrap();
        let current = TargetIdentity::Device {
            identity: "monitor-b".into(),
        };
        *primary.lock().unwrap() = current.clone();
        let card = display_selection_card(&service);
        assert_eq!(card.target, Some(current));
        assert!(!card.owned);
        assert!(service.controller.owners().is_empty());
    }
    #[test]
    fn display_selection_failed_undo_retains_exact_owned_monitor() {
        let (mut service, setting, primary) = display_selection_fixture();
        let original = primary.lock().unwrap().clone();
        let feature = setting.descriptor().id;
        service
            .apply_tweak(feature.clone(), original.clone())
            .unwrap();
        service.confirm_display_tweak().unwrap();
        *primary.lock().unwrap() = TargetIdentity::Device {
            identity: "monitor-b".into(),
        };
        *setting.fail_at.lock().unwrap() = Some(ResourceId::new("value").unwrap());
        assert_eq!(service.undo_tweak(&feature), Err(SaveError::Recovering));
        let card = display_selection_card(&service);
        assert_eq!(card.target, Some(original));
        assert!(card.owned);
        assert_eq!(card.status, Some(ModuleStatus::PartialFailure));
        assert!(!card.ready);
    }

    #[test]
    fn compact_missing_target_keeps_the_actual_read_failure() {
        let module = Setting::new("simple_tweak", false);
        let mut registry = Registry::default();
        registry.register(module.clone()).unwrap();
        let service = super::super::Tuning::from_registry(registry)
            .headless(AppStateStorage::test_fixture(|_| {}), Box::new(NoSave));
        let reason = "Cannot verify the Windows account";
        let template = ModuleCard {
            feature: module.descriptor().id,
            title: "Test tweak".into(),
            enabled: false,
            mode: ActivationMode::Manual,
            config: module.default_config(),
            status: Some(ModuleStatus::Unmanaged),
            capability: Capability::Denied(reason.into()),
            targets: vec![],
            observed: Some(ObservationSummary {
                summary: reason.into(),
                effect: Effect::Immediate,
            }),
        };
        let templates = BTreeMap::from([(template.feature.clone(), template)]);
        let cards = tweak_cards(&service, &templates, true);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].capability, Capability::Denied(reason.into()));
        assert_eq!(cards[0].observed.as_ref().unwrap().summary, reason);
        assert!(cards[0].target.is_none());
        assert!(
            !cards[0].ready,
            "A diagnostic read supplies no write target"
        );
        let initial = tweak_cards(&service, &BTreeMap::new(), false);
        assert_eq!(
            initial[0].capability,
            Capability::Unsupported("Reading current setting…".into())
        );
    }

    #[test]
    fn manual_display_retry_is_bounded_and_closing_bypasses_the_retry_gap() {
        let now = Instant::now();
        let mut next_retry = now;
        assert!(!display_retry_due(
            now,
            false,
            Some(now + Duration::from_secs(10)),
            &mut next_retry
        ));
        assert!(display_retry_due(now, false, Some(now), &mut next_retry));
        assert!(!display_retry_due(
            now + Duration::from_millis(999),
            false,
            Some(now),
            &mut next_retry
        ));
        assert!(display_retry_due(
            now + Duration::from_secs(1),
            false,
            Some(now),
            &mut next_retry
        ));
        assert!(display_retry_due(
            now + Duration::from_secs(1),
            true,
            Some(now + Duration::from_secs(10)),
            &mut next_retry
        ));
        assert!(!display_retry_due(now, true, None, &mut next_retry));
    }

    #[test]
    fn manual_display_shutdown_retains_recovery_required_when_close_restore_cannot_save() {
        struct FailedSave;
        impl super::super::StatePersistence for FailedSave {
            fn save(
                &mut self,
                _: &mut AppStateStorage,
            ) -> Result<super::super::service::SavedState, SaveError> {
                Err(SaveError::Storage)
            }
        }
        let mut service = super::super::Tuning::from_registry(Registry::default())
            .headless(AppStateStorage::test_fixture(|_| {}), Box::new(FailedSave));
        service.admit_test_modules();
        service.display_confirmation = Some(Instant::now() + Duration::from_secs(60));
        let runtime = TuningRuntime::start(
            service,
            Arc::new(tokio::sync::RwLock::new(RunningApps::default())),
        );
        runtime.begin_shutdown();
        let until = Instant::now() + Duration::from_secs(2);
        while !runtime.snapshot().shutdown_complete && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        let terminal = runtime.snapshot();
        assert!(terminal.shutdown_complete);
        assert!(
            terminal.display_confirmation.is_some(),
            "failed close restore retains its pending display evidence"
        );
        assert_eq!(
            terminal.shutdown_outcome,
            Some(ShutdownResult::RecoveryRequired)
        );
    }

    struct SavedExeCard(Arc<Setting>, TargetIdentity);
    impl ModuleEditor for SavedExeCard {
        fn availability(&self) -> super::super::platforms::TweakAvailability {
            self.0.availability()
        }
        fn tweak(&self) -> Option<super::super::presentation::TweakSpec> {
            Some(super::super::presentation::TweakSpec {
                order: 1,
                section: super::super::presentation::TweakSection::Hardware,
                title: "Saved EXE preference",
                scope: "One saved EXE",
                apply_label: "Apply",
                help: "Fixture",
            })
        }
        fn selects_saved_exe(&self) -> bool {
            true
        }
        fn feature(&self) -> FeatureId {
            self.0.descriptor().id
        }
        fn default_config(&self) -> ModuleEnvelope {
            self.0.config(12)
        }
        fn accepts_config(&self, config: &ModuleEnvelope) -> bool {
            self.0.accepts_config(config)
        }
        fn show(&self, _: &mut eframe::egui::Ui, _: &mut ModuleEnvelope) -> Result<bool, ()> {
            Ok(false)
        }
        fn format_baseline(&self, fields: &[(ResourceId, Vec<u8>)]) -> Option<String> {
            fields
                .iter()
                .find(|(field, _)| field.as_str() == "value")
                .map(|(_, bytes)| format!("{:?}", bytes))
        }
    }
    impl Module for SavedExeCard {
        fn descriptor(&self) -> ModuleDescriptor {
            let mut descriptor = self.0.descriptor();
            descriptor.target_scopes = vec![TargetScope::PnpDevice];
            descriptor
        }
        fn editor(&self) -> &dyn ModuleEditor {
            self
        }
        fn targets(&self) -> Vec<TargetIdentity> {
            vec![self.1.clone()]
        }
        fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
            self.0.observe(target)
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

    #[test]
    fn manual_tweak_snapshot_retains_disabled_exe_owner_target_baseline_and_undo_after_failed_restore(
    ) {
        let setting = Setting::new("gpu_snapshot_fixture", false);
        let target = TargetIdentity::Device {
            identity: "saved-exe-fixture".into(),
        };
        let module = Arc::new(SavedExeCard(setting.clone(), target.clone()));
        let feature = module.descriptor().id;
        let mut registry = Registry::default();
        registry.register(module).unwrap();
        let mut service = super::super::Tuning::from_registry(registry)
            .headless(AppStateStorage::test_fixture(|_| {}), Box::new(NoSave));
        service.admit_test_modules();
        let profile = service
            .create_profile(
                "Windows Tweaks [manual v1]".into(),
                vec![ModulePreference {
                    feature: feature.clone(),
                    enabled: true,
                    mode: ActivationMode::Manual,
                    config: setting.config(12),
                }],
            )
            .unwrap();
        service
            .activate_manual(profile, feature.clone(), target.clone())
            .unwrap();
        service
            .tweak_targets
            .insert(feature.clone(), target.clone());
        *setting.fail_at.lock().unwrap() = Some(ResourceId::new("value").unwrap());
        assert_eq!(service.disable_profile(profile), Err(SaveError::Recovering));
        // Transient selection disappears at restart; persisted disablement and
        // the unresolved field owner must still provide an actionable card.
        service.tweak_targets.clear();
        let TuningLoad::Known(config) = service.state().tuning.decode() else {
            panic!("saved fixture")
        };
        assert!(
            !config
                .profiles
                .iter()
                .find(|saved| saved.id == profile)
                .unwrap()
                .modules[0]
                .enabled
        );
        let cards = tweak_cards(&service, &BTreeMap::new(), true);
        let card = cards.iter().find(|card| card.feature == feature).unwrap();
        assert_eq!(card.target, Some(target));
        assert!(card.owned, "the retained owner must keep Undo visible");
        assert_eq!(card.baseline.as_deref(), Some("[7]"));
        assert_eq!(card.status, Some(ModuleStatus::PartialFailure));
        assert!(!card.ready);
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
