//! Immutable execution facts. Completeness is not Game or resource authority.
use crate::app::{
    features::rules::RulesContext,
    models::{AppRuntimeKey, AppStateStorage, RunningApps},
    shared::ids::{GroupId, RuleId},
};
use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::RwLock as TokioRwLock;

const MAX_MEMBERS: usize = 256;
const MAX_PROCESSES: usize = 1024;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnknownReason {
    Unavailable,
    Busy,
    Overflow,
    InvalidMembership,
    RevisionChanged,
    IncompleteEnumeration,
    #[cfg(any(windows, test))]
    Denied,
    InstanceChanged,
    #[cfg(windows)]
    ObservedExit,
    ForegroundRace,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigurationRevision {
    rules: Vec<ConfiguredRule>,
    tuning: Option<Arc<str>>,
    byte_len: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfiguredRule {
    group: GroupId,
    rule: RuleId,
    key: AppRuntimeKey,
    descendants: bool,
    additional: Vec<String>,
    background_image: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    pub key: AppRuntimeKey,
    pub group: GroupId,
    pub rule: RuleId,
    pub instances: Vec<ProcessFacts>,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProcessFacts {
    pub pid: u32,
    pub token: u64,
    pub image: Arc<str>,
    #[cfg(windows)]
    pub native: Option<Arc<os_api::tuning::TuningProcessIdentity>>,
}
impl std::fmt::Debug for ProcessFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProcessFacts(opaque)")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionObservation {
    Complete {
        generation: u64,
        config: Arc<ConfigurationRevision>,
        members: Vec<Member>,
        foreground: ProcessFacts,
        names: std::collections::BTreeMap<u32, String>,
    },
    Unknown(UnknownReason),
}
impl ExecutionObservation {
    pub(crate) fn matches_state(&self, state: &AppStateStorage) -> bool {
        matches!(self, Self::Complete { config, .. } if configuration(state).is_ok_and(|actual| &actual == config.as_ref()))
    }
    #[cfg(test)]
    pub(crate) fn fixture(
        state: &AppStateStorage,
        generation: u64,
        members: Vec<Member>,
        foreground: ProcessFacts,
    ) -> Self {
        Self::Complete {
            generation,
            config: Arc::new(configuration(state).unwrap()),
            members,
            foreground,
            names: Default::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ForegroundSample {
    pub window: usize,
    pub pid: u32,
}
trait ObservationOs {
    fn enumerate(&mut self) -> Result<BTreeSet<u32>, UnknownReason>;
    fn names(&self) -> std::collections::BTreeMap<u32, String> {
        Default::default()
    }
    fn game_candidate(&mut self, _: &ProcessFacts) -> Result<bool, UnknownReason> {
        Ok(true)
    }
    fn inspect(&mut self, pid: u32) -> Result<ProcessFacts, UnknownReason>;
    fn foreground(&mut self) -> Result<ForegroundSample, UnknownReason>;
    fn recheck(&mut self) -> Result<(), UnknownReason> {
        Ok(())
    }
}
pub(crate) struct ExecutionBridge {
    latest: Arc<ExecutionObservation>,
    requested: bool,
    next: Instant,
}
impl Default for ExecutionBridge {
    fn default() -> Self {
        Self {
            latest: Arc::new(ExecutionObservation::Unknown(UnknownReason::Unavailable)),
            requested: true,
            next: Instant::now(),
        }
    }
}
impl ExecutionBridge {
    pub(crate) fn latest(&self) -> Arc<ExecutionObservation> {
        self.latest.clone()
    }
    pub(crate) fn request_refresh(&mut self) {
        self.requested = true;
    }
    fn refresh_with(
        &mut self,
        now: Instant,
        running: &Arc<TokioRwLock<RunningApps>>,
        state: &Arc<RwLock<AppStateStorage>>,
        os: &mut impl ObservationOs,
    ) {
        if !self.requested && now < self.next {
            return;
        }
        self.requested = false;
        self.next = now + REFRESH_INTERVAL;
        let result = observe(running, state, os);
        self.latest = match result {
            Ok(candidate @ ExecutionObservation::Complete { .. }) => {
                let publish = || {
                    let config_guard = state.try_read().map_err(|_| UnknownReason::Busy)?;
                    let membership_guard = running.try_read().map_err(|_| UnknownReason::Busy)?;
                    let ExecutionObservation::Complete {
                        generation, config, ..
                    } = &candidate
                    else {
                        unreachable!()
                    };
                    if membership_guard.membership_generation() != Some(*generation)
                        || &configuration(&config_guard)? != config.as_ref()
                    {
                        return Err(UnknownReason::RevisionChanged);
                    }
                    // Publication happens while both final read guards are held.
                    self.latest = Arc::new(candidate);
                    Ok(self.latest.clone())
                };
                publish().unwrap_or_else(|reason| Arc::new(ExecutionObservation::Unknown(reason)))
            }
            Ok(_) => unreachable!(),
            Err(reason) => Arc::new(ExecutionObservation::Unknown(reason)),
        };
    }
    pub(crate) fn refresh(
        &mut self,
        now: Instant,
        running: &Arc<TokioRwLock<RunningApps>>,
        state: &Arc<RwLock<AppStateStorage>>,
    ) {
        self.refresh_with(now, running, state, &mut NativeOs::default());
    }
}
fn configuration(state: &AppStateStorage) -> Result<ConfigurationRevision, UnknownReason> {
    if state.groups.len() > MAX_MEMBERS
        || state.groups.iter().map(|g| g.programs.len()).sum::<usize>() > MAX_MEMBERS
    {
        return Err(UnknownReason::Overflow);
    }
    let rules = RulesContext::from_storage(state);
    let selected_background = super::background_policy::selected_rules(state);
    let mut rows = Vec::new();
    let mut ids = BTreeSet::new();
    let mut bytes = state.tuning.raw().map_or(0, str::len);
    for (group_index, group) in state.groups.iter().enumerate() {
        for (rule_index, app) in group.programs.iter().enumerate() {
            let group = rules
                .group_id_for_index(group_index)
                .ok_or(UnknownReason::InvalidMembership)?;
            let rule = rules
                .rule_id_for_index(group_index, rule_index)
                .ok_or(UnknownReason::InvalidMembership)?;
            if !ids.insert(rule.0.clone()) {
                return Err(UnknownReason::InvalidMembership);
            }
            let key = app.get_key();
            bytes = bytes
                .checked_add(key.as_str().len() + group.0.len() + rule.0.len())
                .ok_or(UnknownReason::Overflow)?;
            if app.additional_processes.len() > MAX_PROCESSES {
                return Err(UnknownReason::Overflow);
            }
            for name in &app.additional_processes {
                bytes = bytes
                    .checked_add(name.len())
                    .ok_or(UnknownReason::Overflow)?;
            }
            if bytes > MAX_BYTES {
                return Err(UnknownReason::Overflow);
            }
            let background_image = selected_background
                .contains(&rule.0)
                .then(|| {
                    app.bin_path()
                        .map(|path| path.to_string_lossy().into_owned())
                })
                .flatten();
            if let Some(image) = &background_image {
                bytes = bytes
                    .checked_add(image.len())
                    .ok_or(UnknownReason::Overflow)?;
            }
            rows.push(ConfiguredRule {
                group,
                rule,
                key,
                descendants: app.manage_descendants,
                additional: app.additional_processes.clone(),
                background_image,
            });
        }
    }
    if bytes > MAX_BYTES {
        return Err(UnknownReason::Overflow);
    }
    Ok(ConfigurationRevision {
        rules: rows,
        tuning: state.tuning.raw().map(Arc::from),
        byte_len: bytes,
    })
}
fn observe(
    running: &Arc<TokioRwLock<RunningApps>>,
    state: &Arc<RwLock<AppStateStorage>>,
    os: &mut impl ObservationOs,
) -> Result<ExecutionObservation, UnknownReason> {
    let config = Arc::new(configuration(
        &*state.try_read().map_err(|_| UnknownReason::Busy)?,
    )?);
    let (generation, copied) = {
        let running = running.try_read().map_err(|_| UnknownReason::Busy)?;
        let generation = running
            .membership_generation()
            .ok_or(UnknownReason::Overflow)?;
        if running.apps.len() > MAX_MEMBERS {
            return Err(UnknownReason::Overflow);
        }
        let mut copied = Vec::new();
        let mut count = 0usize;
        for (key, app) in &running.apps {
            count = count
                .checked_add(app.pids.len())
                .ok_or(UnknownReason::Overflow)?;
            if count > MAX_PROCESSES {
                return Err(UnknownReason::Overflow);
            }
            if app.pids.is_empty()
                || app.pids.len() != app.pid_instance_tokens.len()
                || !config.rules.iter().any(|rule| {
                    rule.key == *key && rule.group == app.group_id && rule.rule == app.rule_id
                })
            {
                return Err(UnknownReason::InvalidMembership);
            }
            let mut pids = BTreeSet::new();
            let mut instances = Vec::new();
            for pid in &app.pids {
                let token = app
                    .pid_instance_tokens
                    .get(pid)
                    .copied()
                    .filter(|token| *token != 0)
                    .ok_or(UnknownReason::InvalidMembership)?;
                if *pid == 0 || !pids.insert(*pid) {
                    return Err(UnknownReason::InvalidMembership);
                }
                instances.push((*pid, token));
            }
            copied.push((
                key.clone(),
                app.group_id.clone(),
                app.rule_id.clone(),
                instances,
            ));
        }
        (generation, copied)
    };
    let enumeration = os.enumerate()?;
    if enumeration.len() > MAX_PROCESSES {
        return Err(UnknownReason::Overflow);
    }
    let mut members = Vec::new();
    let mut bytes = config.byte_len;
    let names = os.names();
    for (key, group, rule, instances) in copied {
        bytes = bytes
            .checked_add(key.as_str().len() + group.0.len() + rule.0.len())
            .ok_or(UnknownReason::Overflow)?;
        let mut facts = Vec::new();
        for (pid, token) in instances {
            if !enumeration.contains(&pid) {
                return Err(UnknownReason::IncompleteEnumeration);
            }
            let actual = os.inspect(pid)?;
            if actual.pid != pid || actual.token != token {
                return Err(UnknownReason::InstanceChanged);
            }
            bytes = bytes
                .checked_add(fact_bytes(&actual))
                .ok_or(UnknownReason::Overflow)?;
            if bytes > MAX_BYTES {
                return Err(UnknownReason::Overflow);
            }
            facts.push(actual);
        }
        members.push(Member {
            key,
            group,
            rule,
            instances: facts,
        });
    }
    // Read-only discovery for selected saved EXEs also works with the classic
    // monitor paused. Reuse this enumeration; never insert into RunningApps.
    for rule in config
        .rules
        .iter()
        .filter(|rule| rule.background_image.is_some())
    {
        let image = rule.background_image.as_ref().unwrap();
        let basename = std::path::Path::new(image)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(UnknownReason::InvalidMembership)?;
        for (&pid, name) in &names {
            if !name.eq_ignore_ascii_case(basename)
                || members.iter().any(|member| {
                    member.rule == rule.rule
                        && member.instances.iter().any(|facts| facts.pid == pid)
                })
            {
                continue;
            }
            let facts = os.inspect(pid)?;
            if !facts.image.eq_ignore_ascii_case(image) || !os.game_candidate(&facts)? {
                continue;
            }
            bytes = bytes
                .checked_add(fact_bytes(&facts))
                .ok_or(UnknownReason::Overflow)?;
            if bytes > MAX_BYTES {
                return Err(UnknownReason::Overflow);
            }
            if let Some(member) = members.iter_mut().find(|member| member.rule == rule.rule) {
                member.instances.push(facts);
            } else {
                members.push(Member {
                    key: rule.key.clone(),
                    group: rule.group.clone(),
                    rule: rule.rule.clone(),
                    instances: vec![facts],
                });
            }
        }
    }
    let first = os.foreground()?;
    if first.window == 0 || first.pid == 0 {
        return Err(UnknownReason::ForegroundRace);
    }
    let foreground = os.inspect(first.pid)?;
    if foreground.pid != first.pid || foreground.token == 0 || first != os.foreground()? {
        return Err(UnknownReason::ForegroundRace);
    }
    if bytes
        .checked_add(fact_bytes(&foreground))
        .is_none_or(|bytes| bytes > MAX_BYTES)
    {
        return Err(UnknownReason::Overflow);
    }
    os.recheck()?;
    if first != os.foreground()? {
        return Err(UnknownReason::ForegroundRace);
    }
    Ok(ExecutionObservation::Complete {
        generation,
        config,
        members,
        foreground,
        names: names
            .into_iter()
            .filter(|(_, name)| os_api::background::candidate_role(name).is_some())
            .collect(),
    })
}
fn fact_bytes(fact: &ProcessFacts) -> usize {
    let bytes = fact.image.len() + 12;
    #[cfg(windows)]
    if let Some(native) = &fact.native {
        return bytes + native.image_path().len() + native.token().sid().len() + 24;
    }
    bytes
}
#[derive(Default)]
struct NativeOs {
    names: std::collections::BTreeMap<u32, String>,
    #[cfg(windows)]
    own: Option<os_api::tuning::TuningProcessIdentity>,
    #[cfg(windows)]
    guards: Vec<os_api::tuning::TuningProcessLifetimeGuard>,
}
impl ObservationOs for NativeOs {
    fn game_candidate(&mut self, facts: &ProcessFacts) -> Result<bool, UnknownReason> {
        #[cfg(windows)]
        {
            if self.own.is_none() {
                self.own = Some(
                    os_api::OS::tuning_process_identity(std::process::id())
                        .map_err(|_| UnknownReason::Denied)?,
                );
            }
            let own = self.own.as_ref().unwrap().token();
            let Some(native) = &facts.native else {
                return Ok(false);
            };
            let token = native.token();
            Ok(token.session_id() != 0
                && token.session_id() == own.session_id()
                && token.sid() == own.sid()
                && token.authentication_id() == own.authentication_id())
        }
        #[cfg(not(windows))]
        {
            let _ = facts;
            Ok(false)
        }
    }
    fn enumerate(&mut self) -> Result<BTreeSet<u32>, UnknownReason> {
        let tree = os_api::OS::snapshot_process_tree()
            .map_err(|_| UnknownReason::IncompleteEnumeration)?;
        if tree.names.len() > MAX_PROCESSES || tree.names.values().any(|name| name.len() > 1024) {
            return Err(UnknownReason::Overflow);
        }
        self.names = tree.names.into_iter().collect();
        Ok(self.names.keys().copied().collect())
    }
    fn names(&self) -> std::collections::BTreeMap<u32, String> {
        self.names.clone()
    }
    fn inspect(&mut self, pid: u32) -> Result<ProcessFacts, UnknownReason> {
        #[cfg(windows)]
        {
            let native =
                os_api::OS::tuning_process_identity(pid).map_err(|_| UnknownReason::Denied)?;
            let guard = os_api::OS::tuning_open_process_lifetime(&native)
                .map_err(|_| UnknownReason::InstanceChanged)?;
            let facts = ProcessFacts {
                pid: native.pid(),
                token: native.creation_time(),
                image: Arc::from(native.image_path()),
                native: Some(Arc::new(native)),
            };
            self.guards.push(guard);
            Ok(facts)
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Err(UnknownReason::Unavailable)
        }
    }
    fn foreground(&mut self) -> Result<ForegroundSample, UnknownReason> {
        crate::app::adapters::os::foreground_process()
            .map(|(window, pid)| ForegroundSample { window, pid })
            .map_err(|_| UnknownReason::ForegroundRace)
    }
    fn recheck(&mut self) -> Result<(), UnknownReason> {
        #[cfg(windows)]
        for guard in &mut self.guards {
            if guard.poll().map_err(|_| UnknownReason::Denied)?
                != os_api::tuning::TuningProcessLifetime::VerifiedLive
            {
                return Err(UnknownReason::ObservedExit);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::{AppToRun, CoreGroup};
    #[test]
    fn background_selected_saved_exe_is_discovered_without_classic_monitor_membership() {
        struct Named(FakeOs, bool);
        impl ObservationOs for Named {
            fn enumerate(&mut self) -> Result<BTreeSet<u32>, UnknownReason> {
                self.0.enumerate()
            }
            fn names(&self) -> std::collections::BTreeMap<u32, String> {
                [(42, "fixture.exe".into())].into()
            }
            fn game_candidate(&mut self, _: &ProcessFacts) -> Result<bool, UnknownReason> {
                Ok(self.1)
            }
            fn inspect(&mut self, pid: u32) -> Result<ProcessFacts, UnknownReason> {
                self.0.inspect(pid)
            }
            fn foreground(&mut self) -> Result<ForegroundSample, UnknownReason> {
                self.0.foreground()
            }
        }
        let (running, state, os) = fixture();
        running
            .try_write()
            .unwrap()
            .mutate_membership(|running| running.apps.clear());
        let mut saved = state.write().unwrap();
        saved.groups[0].programs[0] = AppToRun::new_path(
            "fixture.exe".into(),
            vec![],
            "fixture.exe".into(),
            os_api::PriorityClass::Normal,
            false,
        );
        let rules = RulesContext::from_storage(&saved);
        let config = super::super::tweaks::background_workload::Config {
            games: vec![super::super::tweaks::background_workload::GameRule {
                group: rules.group_id_for_index(0).unwrap().0,
                rule: rules.rule_id_for_index(0, 0).unwrap().0,
            }],
            ..Default::default()
        };
        let mut tuning = super::super::config::TuningConfig::default();
        tuning.next_profile = 2;
        tuning.profiles.push(super::super::config::Profile {
            version: 1,
            id: super::super::platform::contract::ProfileId::new(1).unwrap(),
            name: "test".into(),
            modules: vec![super::super::config::ModulePreference {
                feature: super::super::platform::contract::FeatureId::new("background_workload")
                    .unwrap(),
                enabled: true,
                mode: super::super::platform::contract::ActivationMode::Session,
                config: config.envelope(),
            }],
        });
        saved.tuning = super::super::config::TuningEnvelope::from_config(&tuning).unwrap();
        drop(saved);
        let mut named = Named(os, true);
        let observation = observe(&running, &state, &mut named).unwrap();
        assert!(
            matches!(observation,ExecutionObservation::Complete{members,..} if members.len()==1 && members[0].instances[0].pid==42)
        );
        assert!(
            running.try_read().unwrap().apps.is_empty(),
            "discovery must not enforce affinity or change the classic roster"
        );
        named.1 = false;
        let observation = observe(&running, &state, &mut named).unwrap();
        assert!(
            matches!(observation,ExecutionObservation::Complete{members,..} if members.is_empty()),
            "another logon cannot become a game trigger"
        );
    }
    struct FakeOs {
        error: Option<UnknownReason>,
        focus: Vec<ForegroundSample>,
        calls: usize,
        hook: Option<Box<dyn FnMut()>>,
    }
    fn facts(pid: u32) -> ProcessFacts {
        ProcessFacts {
            pid,
            token: 100 + pid as u64,
            image: Arc::from("fixture.exe"),
            #[cfg(windows)]
            native: None,
        }
    }
    impl ObservationOs for FakeOs {
        fn enumerate(&mut self) -> Result<BTreeSet<u32>, UnknownReason> {
            if self.error == Some(UnknownReason::IncompleteEnumeration) {
                return Err(UnknownReason::IncompleteEnumeration);
            }
            Ok([42, 99].into())
        }
        fn inspect(&mut self, pid: u32) -> Result<ProcessFacts, UnknownReason> {
            self.calls += 1;
            if let Some(hook) = &mut self.hook {
                hook();
            }
            if self.error == Some(UnknownReason::Denied) {
                return Err(UnknownReason::Denied);
            }
            Ok(facts(pid))
        }
        fn foreground(&mut self) -> Result<ForegroundSample, UnknownReason> {
            Ok(if self.focus.len() > 1 {
                self.focus.remove(0)
            } else {
                self.focus[0]
            })
        }
    }
    fn fixture() -> (
        Arc<TokioRwLock<RunningApps>>,
        Arc<RwLock<AppStateStorage>>,
        FakeOs,
    ) {
        let mut state = AppStateStorage::test_fixture(|_| {});
        let app = AppToRun::new_installed(
            "fixture".into(),
            "fixture".into(),
            os_api::PriorityClass::Normal,
            false,
        );
        state.groups.push(CoreGroup {
            name: "fixture".into(),
            cores: vec![],
            programs: vec![app.clone()],
            is_hidden: false,
            run_all_button: false,
        });
        let rules = RulesContext::from_storage(&state);
        let mut running = RunningApps::default();
        running.mutate_membership(|running| {
            running.add_app(
                &app.get_key(),
                42,
                rules.group_id_for_index(0).unwrap(),
                rules.rule_id_for_index(0, 0).unwrap(),
            );
            running
                .apps
                .get_mut(&app.get_key())
                .unwrap()
                .pid_instance_tokens
                .insert(42, 142);
        });
        (
            Arc::new(TokioRwLock::new(running)),
            Arc::new(RwLock::new(state)),
            FakeOs {
                error: None,
                focus: vec![ForegroundSample { window: 1, pid: 99 }],
                calls: 0,
                hook: None,
            },
        )
    }
    #[test]
    fn p3_bridge_complete_positive_other_focus_and_dropped_wake_converge() {
        let (running, state, mut os) = fixture();
        let mut bridge = ExecutionBridge::default();
        let now = Instant::now();
        bridge.refresh_with(now, &running, &state, &mut os);
        assert!(
            matches!(bridge.latest().as_ref(), ExecutionObservation::Complete { members, foreground, .. } if members.len()==1 && foreground.pid==99)
        );
        let old = bridge.latest();
        running.try_write().unwrap().mutate_membership(|running| {
            running.apps.clear();
        });
        bridge.refresh_with(now + REFRESH_INTERVAL, &running, &state, &mut os);
        assert!(
            matches!(bridge.latest().as_ref(), ExecutionObservation::Complete { members, .. } if members.is_empty())
        );
        assert!(
            matches!(old.as_ref(), ExecutionObservation::Complete { members, .. } if members.len()==1)
        );
        bridge.request_refresh();
        bridge.request_refresh();
        let before = os.calls;
        bridge.refresh_with(now + REFRESH_INTERVAL, &running, &state, &mut os);
        bridge.refresh_with(now + REFRESH_INTERVAL, &running, &state, &mut os);
        assert_eq!(os.calls, before + 1);
    }
    #[test]
    fn p3_bridge_denied_incomplete_and_foreground_race_are_unknown() {
        for reason in [
            UnknownReason::Denied,
            UnknownReason::IncompleteEnumeration,
            UnknownReason::ForegroundRace,
        ] {
            let (running, state, mut os) = fixture();
            os.error = Some(reason);
            if reason == UnknownReason::ForegroundRace {
                os.focus.push(ForegroundSample { window: 2, pid: 42 });
            }
            let mut bridge = ExecutionBridge::default();
            bridge.refresh_with(Instant::now(), &running, &state, &mut os);
            assert_eq!(*bridge.latest(), ExecutionObservation::Unknown(reason));
        }
    }
    #[test]
    fn p3_bridge_membership_and_configuration_inspection_races_are_unknown() {
        for config_race in [false, true] {
            let (running, state, mut os) = fixture();
            let changed_running = running.clone();
            let changed_state = state.clone();
            os.hook = Some(Box::new(move || {
                if config_race {
                    changed_state.write().unwrap().groups[0].programs[0].manage_descendants = true;
                } else {
                    changed_running
                        .try_write()
                        .unwrap()
                        .mutate_membership(|running| {
                            running.apps.clear();
                        });
                }
            }));
            let mut bridge = ExecutionBridge::default();
            bridge.refresh_with(Instant::now(), &running, &state, &mut os);
            assert_eq!(
                *bridge.latest(),
                ExecutionObservation::Unknown(UnknownReason::RevisionChanged)
            );
        }
    }
    #[test]
    fn p3_bridge_busy_invalid_missing_tokens_and_capacity_are_explicit_unknown() {
        for case in 0..4 {
            let (running, state, mut os) = fixture();
            let mut bridge = ExecutionBridge::default();
            let expected = match case {
                0 => {
                    let _writer = running.try_write().unwrap();
                    bridge.refresh_with(Instant::now(), &running, &state, &mut os);
                    assert_eq!(
                        *bridge.latest(),
                        ExecutionObservation::Unknown(UnknownReason::Busy)
                    );
                    continue;
                }
                1 => {
                    running.try_write().unwrap().mutate_membership(|running| {
                        running
                            .apps
                            .values_mut()
                            .next()
                            .unwrap()
                            .pid_instance_tokens
                            .clear();
                    });
                    UnknownReason::InvalidMembership
                }
                2 => {
                    state.write().unwrap().groups[0].programs[0].additional_processes =
                        vec!["x".repeat(MAX_BYTES + 1)];
                    UnknownReason::Overflow
                }
                _ => {
                    running.try_write().unwrap().mutate_membership(|running| {
                        running.apps.values_mut().next().unwrap().pids =
                            (1..=MAX_PROCESSES as u32 + 1).collect();
                    });
                    UnknownReason::Overflow
                }
            };
            bridge.refresh_with(Instant::now(), &running, &state, &mut os);
            assert_eq!(*bridge.latest(), ExecutionObservation::Unknown(expected));
        }
    }
    // Same app test executable, explicit opt-in read only. No source/journal/OS
    // writes or native tuning scope are constructed by this characterization.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires explicit read-only host foreground validation"]
    fn p3_readonly_foreground_characterization() {
        let mut os = NativeOs::default();
        let first = os.foreground().expect("actual foreground available");
        let facts = os
            .inspect(first.pid)
            .expect("single-object foreground facts");
        assert_eq!(facts.pid, first.pid);
        assert!(facts.token != 0 && facts.native.is_some());
        assert_eq!(os.foreground().unwrap(), first);
        os.recheck().expect("same retained object verified live");
        assert_eq!(os.foreground().unwrap(), first);
        println!(
            "valid_foreground=true matched_window_pid=true single_instance=true verified_live=true"
        );
    }
}
