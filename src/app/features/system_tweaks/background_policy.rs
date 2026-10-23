//! Session-only background policy, advanced by the existing tuning worker.
#[cfg(test)]
use super::tweaks::background_workload::GameRule;
use super::{
    config::{ModulePreference, Profile, TuningLoad},
    execution_bridge::{ExecutionObservation, ProcessFacts},
    persistence::ActivationFacts,
    platform::{
        contract::*,
        controller::{LifetimeEvidence, OwnerKey, RecoveryIntent, SessionSource},
    },
    service::{HeadlessTuning, SaveError},
    tweaks::background_workload::{configured, resolve_pool, Config, CpuMode, ID},
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) const PROFILE_NAME: &str = "Background workload [session v1]";
#[cfg(any(windows, test))]
const PREVIEW_TIME: Duration = Duration::from_secs(600);
const SCAN_INTERVAL: Duration = Duration::from_secs(5);
fn feature() -> FeatureId {
    FeatureId::new(ID).unwrap()
}

#[derive(Clone, Debug)]
pub(crate) struct CandidateRow {
    pub role: &'static str,
    pub pid: u32,
    pub status: String,
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub enabled: bool,
    pub config: Config,
    pub status: String,
    pub candidates: Vec<CandidateRow>,
    pub active_games: usize,
    pub changed_targets: usize,
    pub preview_until: Option<Instant>,
    pub inspected: bool,
    pub events: Vec<String>,
    pub topology: Arc<Vec<os_api::background::PlacementCpu>>,
    pub topology_error: Option<String>,
    pub placement_status: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            config: Config::default(),
            status: "Off · no background settings changed".into(),
            candidates: vec![],
            active_games: 0,
            changed_targets: 0,
            preview_until: None,
            inspected: false,
            events: vec![],
            topology: Arc::new(vec![]),
            topology_error: None,
            placement_status: None,
        }
    }
}
struct Preview {
    deadline: Instant,
    facts: ProcessFacts,
}
struct SessionPool {
    requested: Config,
    resolved: Result<Config, String>,
}
#[derive(Default)]
pub(crate) struct Runtime {
    preview: Option<Preview>,
    inspect_requested: bool,
    next_scan: Option<Instant>,
    inventory: Vec<os_api::background::Candidate>,
    resolver: os_api::background::Resolver,
    pools: BTreeMap<(ProfileId, SessionSource), SessionPool>,
    topology_read: Option<Instant>,
    // Only current-process acquisitions enter this map. Retained owners from a
    // previous process are restored rather than silently reactivated.
    acquired: BTreeMap<OwnerKey, super::platform::codec::ModuleEnvelope>,
    pub snapshot: Snapshot,
}
impl Runtime {
    #[cfg(any(windows, test))]
    pub(crate) fn start_preview(
        &mut self,
        facts: ProcessFacts,
        now: Instant,
    ) -> Result<(), SaveError> {
        if self.preview.is_some() || facts.pid == 0 || facts.token == 0 {
            return Err(SaveError::Invalid);
        }
        self.preview = Some(Preview {
            deadline: now + PREVIEW_TIME,
            facts,
        });
        self.inspect_requested = true;
        self.next_scan = None;
        Ok(())
    }
    fn preview_expired(&self, now: Instant) -> bool {
        self.preview
            .as_ref()
            .is_some_and(|preview| now >= preview.deadline)
    }
    pub(crate) fn request_inspect(&mut self) {
        self.inspect_requested = true;
        self.next_scan = None;
    }
    pub(crate) fn stop(&mut self) {
        self.preview = None;
        self.acquired.clear();
        self.pools.clear();
    }
    pub(crate) fn needs_work(&self) -> bool {
        self.inspect_requested || self.preview.is_some()
    }
    fn read_topology(
        &mut self,
        now: Instant,
    ) -> Result<Arc<Vec<os_api::background::PlacementCpu>>, String> {
        if self.topology_read == Some(now) {
            return self
                .snapshot
                .topology_error
                .clone()
                .map_or_else(|| Ok(self.snapshot.topology.clone()), Err);
        }
        self.topology_read = Some(now);
        match os_api::background::placement_topology(None) {
            Ok(topology) if topology.len() <= 512 => {
                self.snapshot.topology = Arc::new(topology);
                self.snapshot.topology_error = None;
                Ok(self.snapshot.topology.clone())
            }
            result => {
                let reason = match result {
                    Ok(_) => {
                        "CPU topology exceeds the local preview's 512 logical CPU limit".into()
                    }
                    Err(error) => error.to_string(),
                };
                self.snapshot.topology = Arc::new(vec![]);
                self.snapshot.topology_error = Some(reason.clone());
                Err(reason)
            }
        }
    }
    fn session_config(
        &mut self,
        profile: ProfileId,
        source: &SessionSource,
        config: &Config,
        now: Instant,
    ) -> Result<Config, String> {
        if config.cpu_mode == CpuMode::PriorityEco {
            return Ok(config.clone());
        }
        let key = (profile, source.clone());
        if let Some(pool) = self
            .pools
            .get(&key)
            .filter(|pool| pool.requested == *config)
        {
            return pool.resolved.clone();
        }
        let resolved = self.read_topology(now).and_then(|topology| {
            resolve_pool(config, &topology).map_err(|reason| {
                format!(
                    "CPU placement unavailable: {reason:?}; priority and EcoQoS remain available"
                )
            })
        });
        if self.pools.len() < 129 || self.pools.contains_key(&key) {
            self.pools.insert(
                key,
                SessionPool {
                    requested: config.clone(),
                    resolved: resolved.clone(),
                },
            );
        } else {
            return Err("Session pool capacity exceeded".into());
        }
        resolved
    }
}

pub(crate) fn policies(service: &HeadlessTuning) -> Vec<(ProfileId, bool, Config)> {
    let TuningLoad::Known(config) = service.state().tuning.decode() else {
        return vec![];
    };
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.modules.iter().filter_map(|preference| {
                (preference.feature.as_str() == ID && preference.mode == ActivationMode::Session)
                    .then(|| {
                        configured(&preference.config)
                            .ok()
                            .map(|config| (profile.id, preference.enabled, config))
                    })
                    .flatten()
            })
        })
        .collect()
}
pub(crate) fn selected_rules(
    state: &crate::app::models::AppStateStorage,
) -> std::collections::BTreeSet<String> {
    let TuningLoad::Known(config) = state.tuning.decode() else {
        return Default::default();
    };
    config
        .profiles
        .iter()
        .flat_map(|profile| &profile.modules)
        .filter(|preference| {
            preference.enabled
                && preference.mode == ActivationMode::Session
                && preference.feature.as_str() == ID
        })
        .filter_map(|preference| configured(&preference.config).ok())
        .flat_map(|config| config.games.into_iter().map(|game| game.rule))
        .collect()
}
pub(crate) fn snapshot(service: &HeadlessTuning) -> Snapshot {
    let mut snapshot = service.background.snapshot.clone();
    snapshot.preview_until = service
        .background
        .preview
        .as_ref()
        .map(|preview| preview.deadline);
    snapshot.changed_targets = service
        .controller
        .owners()
        .iter()
        .filter_map(|owner| match owner {
            OwnerKey::SessionTarget {
                target_pid,
                target_token,
                feature,
                ..
            } if feature.as_str() == ID => Some((*target_pid, *target_token)),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    if let TuningLoad::Known(config) = service.state().tuning.decode() {
        if let Some(preference) = config
            .profiles
            .iter()
            .find(|profile| profile.name == PROFILE_NAME)
            .and_then(|profile| {
                profile
                    .modules
                    .iter()
                    .find(|module| module.feature.as_str() == ID)
            })
        {
            if let Ok(config) = configured(&preference.config) {
                snapshot.config = config;
                snapshot.enabled = preference.enabled;
            }
        }
    }
    if !snapshot.enabled && snapshot.preview_until.is_none() && snapshot.changed_targets == 0 {
        snapshot.status = "Off · no background settings changed".into();
    }
    snapshot
}
pub(crate) fn game_sources(
    config: &Config,
    observation: &ExecutionObservation,
) -> Vec<(SessionSource, ProcessFacts)> {
    let ExecutionObservation::Complete { members, .. } = observation else {
        return vec![];
    };
    members
        .iter()
        .filter(|member| config.games.iter().any(|game| game.rule == member.rule.0))
        .flat_map(|member| {
            member.instances.iter().map(|facts| {
                (
                    SessionSource::Game {
                        group: member.group.0.clone(),
                        rule: member.rule.0.clone(),
                        pid: facts.pid,
                        token: facts.token,
                    },
                    facts.clone(),
                )
            })
        })
        .take(129)
        .collect()
}
fn retention_intent(configured: bool, expired: bool, lifetime: LifetimeEvidence) -> RecoveryIntent {
    if !configured || expired || lifetime == LifetimeEvidence::Exited {
        RecoveryIntent::Restore
    } else if lifetime == LifetimeEvidence::Live {
        RecoveryIntent::Keep
    } else {
        RecoveryIntent::Unknown
    }
}
fn candidate_skip(
    row: &os_api::background::Candidate,
    config: &Config,
    protected: &[u32],
) -> Option<String> {
    if protected.contains(&row.pid) {
        return Some("Game, tracked app or foreground process; excluded".into());
    }
    if config.excluded_roles.contains(&row.role) {
        return Some("Excluded in your settings".into());
    }
    if !row.verified && !row.reason.is_empty() {
        return Some(format!(
            "{} · {}",
            row.reason,
            row.role.qualification_note()
        ));
    }
    if !row.role.base_candidate() {
        return Some("Optional role; not enabled in this version".into());
    }
    if !row.role.mutation_qualified() {
        return Some(row.role.qualification_note().into());
    }
    if !row.verified || row.process.is_none() || row.proof.is_none() {
        return Some(row.reason.clone());
    }
    None
}

impl HeadlessTuning {
    pub(crate) fn configure_background(
        &mut self,
        enabled: bool,
        config: Config,
    ) -> Result<(), SaveError> {
        if !config.valid() || (enabled && config.games.is_empty()) {
            return Err(SaveError::Invalid);
        }
        let choices = game_choices(self.state(), self.rules());
        if enabled
            && config
                .games
                .iter()
                .any(|game| !choices.iter().any(|choice| choice.rule == game.rule))
        {
            return Err(SaveError::Invalid);
        }
        let mut saved = self.configured()?;
        let matching: Vec<_> = saved
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| profile.name == PROFILE_NAME)
            .map(|(index, _)| index)
            .collect();
        if matching.len() > 1 {
            return Err(SaveError::Invalid);
        }
        let preference = ModulePreference {
            feature: feature(),
            enabled,
            mode: ActivationMode::Session,
            config: config.envelope(),
        };
        if let Some(&index) = matching.first() {
            let profile = &mut saved.profiles[index];
            if profile.modules.len() != 1 || profile.modules[0].feature != feature() {
                return Err(SaveError::Invalid);
            }
            profile.modules[0] = preference;
        } else {
            if saved.profiles.len() >= 64 {
                return Err(SaveError::Invalid);
            }
            let id = ProfileId::new(saved.next_profile).map_err(|_| SaveError::Invalid)?;
            saved.next_profile = saved
                .next_profile
                .checked_add(1)
                .ok_or(SaveError::Invalid)?;
            saved.profiles.push(Profile {
                version: 1,
                id,
                name: PROFILE_NAME.into(),
                modules: vec![preference],
            });
        }
        self.save_config(saved)
    }
    pub(crate) fn preview_background(&mut self) -> Result<(), SaveError> {
        self.verify_saved()?;
        if !self.native_admitted || !self.controller.can_admit() {
            return Err(SaveError::Recovering);
        }
        if !matches!(self.state().tuning.decode(), TuningLoad::Known(ref saved) if saved.profiles.iter().any(|profile|profile.name==PROFILE_NAME))
        {
            self.configure_background(false, Config::default())?;
        }
        #[cfg(windows)]
        {
            let native = os_api::OS::tuning_process_identity(std::process::id())
                .map_err(|_| SaveError::Invalid)?;
            let facts = ProcessFacts {
                pid: native.pid(),
                token: native.creation_time(),
                image: Arc::from(native.image_path()),
                native: Some(Arc::new(native)),
            };
            self.background.start_preview(facts, Instant::now())
        }
        #[cfg(not(windows))]
        {
            Err(SaveError::Invalid)
        }
    }
    pub(crate) fn undo_background(&mut self) -> Result<(), SaveError> {
        let mut saved = self.configured()?;
        for profile in &mut saved.profiles {
            for preference in &mut profile.modules {
                if preference.feature.as_str() == ID {
                    preference.enabled = false;
                }
            }
        }
        self.save_config(saved)?;
        self.background.stop();
        let owners: Vec<_> = self
            .controller
            .owners()
            .into_iter()
            .filter(|owner| owner.feature().as_str() == ID)
            .collect();
        for owner in owners {
            if !self
                .controller
                .retire_expired_target(&owner)
                .map_err(|error| self.native_failure(error))?
            {
                self.controller
                    .recover_owner(&owner, RecoveryIntent::Restore)
                    .map_err(|error| self.native_failure(error))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct GameChoice {
    pub group: String,
    pub rule: String,
    pub label: String,
}
pub(crate) fn game_choices(
    state: &crate::app::models::AppStateStorage,
    rules: &crate::app::features::rules::RulesContext,
) -> Vec<GameChoice> {
    state
        .groups
        .iter()
        .enumerate()
        .flat_map(|(gi, group)| {
            group
                .programs
                .iter()
                .enumerate()
                .filter_map(move |(ri, app)| {
                    let path = app.bin_path()?;
                    if !path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                    {
                        return None;
                    }
                    Some(GameChoice {
                        group: rules.group_id_for_index(gi)?.0,
                        rule: rules.rule_id_for_index(gi, ri)?.0,
                        label: format!("{} · {}", app.name, group.name),
                    })
                })
        })
        .take(256)
        .collect()
}

pub(crate) fn tick(service: &mut HeadlessTuning, observation: &ExecutionObservation, now: Instant) {
    let mut runtime = std::mem::take(&mut service.background);
    tick_inner(&mut runtime, service, observation, now);
    service.background = runtime;
}
fn tick_inner(
    runtime: &mut Runtime,
    service: &mut HeadlessTuning,
    observation: &ExecutionObservation,
    now: Instant,
) {
    let policies = policies(service);
    let own_policy=policies.iter().find(|(id,_,_)| matches!(service.state().tuning.decode(),TuningLoad::Known(ref saved) if saved.profiles.iter().any(|p|p.id==*id && p.name==PROFILE_NAME)));
    if let Some((_, enabled, config)) = own_policy {
        runtime.snapshot.enabled = *enabled;
        runtime.snapshot.config = config.clone();
    }
    let expired = runtime.preview_expired(now);
    if expired {
        runtime.preview = None;
    }
    runtime.snapshot.preview_until = runtime.preview.as_ref().map(|preview| preview.deadline);
    let complete = observation.matches_state(service.state());
    let mut sources = vec![];
    let mut capacity_exceeded = false;
    if complete {
        for (profile, enabled, config) in &policies {
            if *enabled {
                for (source, facts) in game_sources(config, observation) {
                    if sources.len() == 128 {
                        capacity_exceeded = true;
                        break;
                    }
                    sources.push((*profile, source, facts, config.clone()));
                }
            }
        }
    }
    runtime.snapshot.active_games = sources.len();
    if let (Some(preview), Some((profile, _, config))) = (&runtime.preview, own_policy) {
        sources.push((
            *profile,
            SessionSource::Preview {
                pid: preview.facts.pid,
                token: preview.facts.token,
            },
            preview.facts.clone(),
            config.clone(),
        ));
    }
    if !service.native_admitted || service.legacy_recovery {
        runtime.snapshot.status =
            "Unavailable · recovery must finish before background control".into();
        return;
    }
    // A retained target lifetime and its game lifetime have separate meanings.
    // Only the target's positive native exit proof can retire its original fields.
    for owner in service.controller.owners().into_iter().filter(|owner| {
        matches!(owner, OwnerKey::SessionTarget { .. }) && owner.feature().as_str() == ID
    }) {
        if service.controller.retire_expired_target(&owner) == Ok(true) {
            runtime.acquired.remove(&owner);
            continue;
        }
        let OwnerKey::SessionTarget {
            profile, source, ..
        } = &owner
        else {
            continue;
        };
        let configured = sources.iter().any(|(id, expected, _, config)| {
            id == profile
                && expected == source
                && runtime.acquired.get(&owner) == Some(&config.envelope())
        });
        // Losing a focus/enumeration sample is not proof that a game exited.
        let lifetime =
            if configured && (complete || matches!(source, SessionSource::Preview { .. })) {
                LifetimeEvidence::Live
            } else if !runtime.acquired.contains_key(&owner)
                || matches!(source, SessionSource::Preview { .. })
            {
                LifetimeEvidence::Unknown
            } else {
                #[cfg(windows)]
                {
                    let (pid, token) = source.instance();
                    if os_api::agent::process_instance_exited(pid, token) {
                        LifetimeEvidence::Exited
                    } else {
                        LifetimeEvidence::Unknown
                    }
                }
                #[cfg(not(windows))]
                {
                    LifetimeEvidence::Unknown
                }
            };
        let enabled = runtime.acquired.contains_key(&owner)
            && match source {
                SessionSource::Preview { .. } => runtime.preview.is_some(),
                SessionSource::Game { rule, .. } => policies.iter().any(|(id, on, config)| {
                    id == profile
                        && *on
                        && config.games.iter().any(|game| &game.rule == rule)
                        && runtime.acquired.get(&owner) == Some(&config.envelope())
                }),
            };
        let intent = retention_intent(
            enabled,
            expired && matches!(source, SessionSource::Preview { .. }),
            lifetime,
        );
        if intent != RecoveryIntent::Keep || service.controller.needs_recovery(&owner) {
            let _ = service.controller.recover_owner(&owner, intent);
        }
        if !service.controller.owners().contains(&owner) {
            runtime.acquired.remove(&owner);
        }
    }
    let work = runtime.inspect_requested || !sources.is_empty();
    if work && runtime.next_scan.is_none_or(|next| now >= next) {
        if runtime.inspect_requested {
            let _ = runtime.read_topology(now);
        }
        runtime.inspect_requested = false;
        runtime.next_scan = Some(now + SCAN_INTERVAL);
        let inventory = match observation {
            ExecutionObservation::Complete { names, .. } => runtime.resolver.inspect_named(names),
            _ => runtime.resolver.inspect(),
        };
        runtime.snapshot.inspected = true;
        runtime.snapshot.candidates.clear();
        match inventory {
            Ok(inventory) => runtime.inventory = inventory,
            Err(error) => {
                runtime.inventory.clear();
                runtime.snapshot.candidates = vec![CandidateRow {
                    role: "Background inventory",
                    pid: 0,
                    status: format!("{error:?}"),
                }];
            }
        }
        let protected: Vec<_> = match observation {
            ExecutionObservation::Complete {
                members,
                foreground,
                ..
            } => members
                .iter()
                .flat_map(|member| member.instances.iter().map(|facts| facts.pid))
                .chain([foreground.pid, std::process::id()])
                .collect(),
            _ => vec![std::process::id()],
        };
        runtime.snapshot.placement_status = None;
        let prepared: Vec<_> = sources
            .iter()
            .map(|(profile, source, facts, config)| {
                let resolved = runtime
                    .session_config(*profile, source, config, now)
                    .unwrap_or_else(|reason| {
                        if runtime.snapshot.placement_status.is_none() {
                            runtime.snapshot.placement_status = Some(reason);
                        }
                        let mut fallback = config.clone();
                        fallback.cpu_mode = CpuMode::PriorityEco;
                        fallback
                    });
                (
                    *profile,
                    source.clone(),
                    facts.clone(),
                    config.clone(),
                    resolved,
                )
            })
            .collect();
        let inventory = std::mem::take(&mut runtime.inventory);
        for row in &inventory {
            let config = own_policy
                .map(|(_, _, config)| config)
                .cloned()
                .unwrap_or_default();
            let skipped = candidate_skip(row, &config, &protected);
            runtime.snapshot.candidates.push(CandidateRow {
                role: row.role.label(),
                pid: row.pid,
                status: skipped
                    .clone()
                    .unwrap_or_else(|| "Eligible · waiting for selected game or preview".into()),
            });
            for (profile, source, facts, config, resolved) in &prepared {
                if capacity_exceeded
                    || candidate_skip(row, config, &protected).is_some()
                    || !service.controller.can_admit()
                {
                    continue;
                }
                let Some(process) = &row.process else {
                    continue;
                };
                let owner = OwnerKey::SessionTarget {
                    profile: *profile,
                    feature: feature(),
                    source: source.clone(),
                    target_pid: process.pid,
                    target_token: process.creation_token,
                };
                if runtime.acquired.get(&owner) == Some(&config.envelope()) {
                    continue;
                }
                let Some(module) = service.registry().get(&feature()).cloned() else {
                    continue;
                };
                if service.verify_saved().is_err() {
                    break;
                }
                let target = TargetIdentity::Process {
                    pid: process.pid,
                    creation_token: process.creation_token,
                    image_path: process.image_path.clone(),
                };
                let activation = ActivationFacts {
                    pid: facts.pid,
                    token: facts.token,
                    image: facts.image.to_string(),
                    trigger: ActivationEligibility::Running,
                };
                let result = service.controller.acquire_session(
                    owner.clone(),
                    module.clone(),
                    target.clone(),
                    resolved.envelope(),
                    activation,
                );
                if let Some(display) = runtime.snapshot.candidates.last_mut() {
                    let summary = match &result {
                        Ok(()) if service.controller.owners().contains(&owner) => {
                            "Changed and verified".into()
                        }
                        Ok(()) => "Already suitable; nothing changed".into(),
                        Err(error) => format!(
                            "Skipped: {:?} · {}",
                            error.kind,
                            error.detail().unwrap_or(error.operation)
                        ),
                    };
                    let notes = module.preflight_notes(&target).join("; ");
                    display.status = format!(
                        "{summary}{}{}",
                        if notes.is_empty() { "" } else { " · " },
                        notes
                    )
                    .chars()
                    .take(1024)
                    .collect();
                }
                if result.is_ok() && service.controller.owners().contains(&owner) {
                    runtime.acquired.insert(owner, config.envelope());
                } else {
                    let _ = service
                        .controller
                        .recover_owner(&owner, RecoveryIntent::Restore);
                }
            }
        }
        runtime.inventory = inventory;
    }
    runtime.snapshot.changed_targets = service
        .controller
        .owners()
        .iter()
        .filter(|owner| owner.feature().as_str() == ID)
        .filter_map(|owner| match owner {
            OwnerKey::SessionTarget {
                target_pid,
                target_token,
                ..
            } => Some((*target_pid, *target_token)),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let owners = service.controller.owners();
    runtime.acquired.retain(|owner, _| owners.contains(owner));
    if complete {
        let keys: std::collections::BTreeSet<_> = sources
            .iter()
            .map(|(profile, source, _, _)| (*profile, source.clone()))
            .collect();
        runtime.pools.retain(|key, _| keys.contains(key));
    } else {
        runtime
            .pools
            .retain(|(profile, source), pool| match source {
                SessionSource::Preview { .. } => runtime.preview.is_some(),
                SessionSource::Game { .. } => policies
                    .iter()
                    .any(|(id, on, config)| id == profile && *on && *config == pool.requested),
            });
    }
    let needs_recovery = service
        .controller
        .owners()
        .iter()
        .filter(|owner| owner.feature().as_str() == ID)
        .any(|owner| {
            !matches!(
                service.controller.status(owner),
                Some(ModuleStatus::VerifiedActive(_))
            )
        });
    let status = if capacity_exceeded {
        "Session capacity exceeded · no new background changes admitted".into()
    } else if needs_recovery {
        "Recovery needs attention · retained settings have not been fully restored".into()
    } else if runtime.snapshot.changed_targets > 0 {
        format!(
            "Active · {} background processes changed",
            runtime.snapshot.changed_targets
        )
    } else if runtime.preview.is_some() {
        "Preview running · no qualified background processes changed".into()
    } else if runtime.snapshot.active_games > 0 {
        "Game running · no qualified background processes changed".into()
    } else if runtime.snapshot.enabled {
        "Armed · waiting for a selected game".into()
    } else {
        "Off · no background settings changed".into()
    };
    if runtime.snapshot.status != status {
        if runtime.snapshot.events.len() == 32 {
            runtime.snapshot.events.remove(0);
        }
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        runtime.snapshot.events.push(format!(
            "{:02}:{:02}:{:02} UTC · {}",
            (seconds % 86400) / 3600,
            (seconds % 3600) / 60,
            seconds % 60,
            status
        ));
        runtime.snapshot.status = status;
    }
}

#[cfg(test)]
mod tests;
