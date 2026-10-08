//! Saved headless transactions. Candidate state and logical IDs publish together.
use super::{
    config::*,
    platform::{
        catalog::Catalog,
        codec::ModuleEnvelope,
        contract::*,
        coordinator::Command,
        runner::{Runner, SavePhase, ShutdownResult, Ticket},
    },
};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::{features::rules::RulesContext, models::AppStateStorage};
use std::time::Duration;
const WAIT: Duration = Duration::from_secs(5);
#[derive(Clone, Copy)]
enum AckAction {
    None,
    Record,
    Prune,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveError {
    Storage,
    Invalid,
    Pending,
    Recovering,
}

/// The selected state storage is invoked outside application/runtime locks.
pub(crate) trait StatePersistence {
    fn save(&mut self, candidate: &mut AppStateStorage) -> Result<SavedState, SaveError>;
}
pub(crate) enum SavedState {
    Unavailable,
    #[cfg(windows)]
    Native(Box<super::platform::configured_intent::NativeConfigurationProof>),
    #[cfg(test)]
    Fixture(Vec<u8>),
}
pub(crate) struct SelectedStatePersistence;
impl StatePersistence for SelectedStatePersistence {
    fn save(&mut self, candidate: &mut AppStateStorage) -> Result<SavedState, SaveError> {
        candidate.try_save_state().map_err(|_| SaveError::Storage)?;
        #[cfg(windows)]
        if let Some(proof) = candidate.native_configuration() {
            return Ok(SavedState::Native(Box::new(proof)));
        }
        Ok(SavedState::Unavailable)
    }
}

pub(crate) struct HeadlessTuning {
    state: AppStateStorage,
    rules: RulesContext,
    persistence: Box<dyn StatePersistence>,
    runner: Option<Runner>,
    tickets: std::collections::BTreeMap<(OwnerId, u64, bool), Ticket>,
    recovering: bool,
    catalog: Catalog,
}
impl HeadlessTuning {
    pub(super) fn new(
        state: AppStateStorage,
        persistence: Box<dyn StatePersistence>,
        catalog: Catalog,
    ) -> Self {
        let rules = RulesContext::from_storage(&state);
        Self {
            state,
            rules,
            persistence,
            runner: None,
            tickets: std::collections::BTreeMap::new(),
            recovering: false,
            catalog,
        }
    }
    pub(crate) fn state(&self) -> &AppStateStorage {
        &self.state
    }
    pub(crate) fn rules(&self) -> &RulesContext {
        &self.rules
    }
    #[cfg(windows)]
    pub(crate) fn start_worker(&mut self) -> Result<(), SaveError> {
        if self.runner.is_some() || self.recovering {
            return Err(SaveError::Pending);
        }
        // Explicit dormant entry: the existing production factory still supplies
        // Unavailable input and native saved handoff remains staged until P4.
        self.runner = Some(
            Runner::start_native(
                self.catalog.clone(),
                &self.state,
                super::platform::runner::RunnerLimits::default(),
            )
            .map_err(|_| SaveError::Pending)?,
        );
        Ok(())
    }
    pub(crate) fn save_base(
        &mut self,
        edit: impl FnOnce(&mut AppStateStorage) -> Result<(), SaveError>,
    ) -> Result<(), SaveError> {
        let mut candidate = self.state.clone();
        edit(&mut candidate)?;
        if candidate.tuning.raw() != self.state.tuning.raw() {
            return Err(SaveError::Invalid);
        }
        if let TuningLoad::Known(config) = self.state.tuning.decode() {
            if !config.associations.is_empty() && candidate.groups.len() < self.state.groups.len() {
                // A deleted positional group may be replaced by an identical neighbor.
                return Err(SaveError::Invalid);
            }
            for association in &config.associations {
                let group = self
                    .rules
                    .group_index_for_id(&association.group)
                    .ok_or(SaveError::Invalid)?;
                if self
                    .rules
                    .rule_index_for_id(group, &association.rule)
                    .is_none()
                    || candidate.groups.get(group).map(|g| &g.programs)
                        != self.state.groups.get(group).map(|g| &g.programs)
                {
                    // Positional base edits cannot transfer an associated logical rule.
                    // The explicit delete/move transactions retain its exact withdrawal.
                    return Err(SaveError::Invalid);
                }
            }
        }
        self.publish(candidate)
    }
    fn publish(&mut self, candidate: AppStateStorage) -> Result<(), SaveError> {
        self.publish_with_ack(candidate, AckAction::None)
    }
    fn publish_with_ack(
        &mut self,
        candidate: AppStateStorage,
        ack_action: AckAction,
    ) -> Result<(), SaveError> {
        let mut rules = self.rules.clone();
        rules.reconcile_with_storage(&candidate);
        self.publish_with_rules(candidate, rules, ack_action)
    }
    fn publish_with_rules(
        &mut self,
        mut candidate: AppStateStorage,
        rules: RulesContext,
        ack_action: AckAction,
    ) -> Result<(), SaveError> {
        if self.recovering {
            return Err(SaveError::Recovering);
        }
        candidate.mark_ready_for_current_schema_save(rules.to_persisted_identities());
        let mut reservation = match &self.runner {
            Some(runner) => {
                let mut reservation = runner
                    .client()
                    .reserve_configuration()
                    .map_err(|_| SaveError::Pending)?;
                if reservation.wait(WAIT) != Ok(SavePhase::Granted) {
                    self.recovering = true;
                    return Err(SaveError::Recovering);
                }
                if reservation.begin_publication(WAIT).is_err() {
                    self.recovering = true;
                    return Err(SaveError::Recovering);
                }
                Some(reservation)
            }
            None => None,
        };
        if !matches!(ack_action, AckAction::None) {
            let checked = (|| {
                let records = reservation
                    .as_ref()
                    .and_then(|r| r.acknowledgements())
                    .ok_or(SaveError::Pending)?;
                let TuningLoad::Known(mut config) = candidate.tuning.decode() else {
                    return Err(SaveError::Invalid);
                };
                if records.records().is_empty() {
                    return Err(SaveError::Pending);
                }
                match ack_action {
                    AckAction::Record => {
                        config.version = 2;
                        config.transitions.get_or_insert_with(Vec::new);
                        config.acknowledgements = Some(records.records().to_vec());
                    }
                    AckAction::Prune => {
                        let saved = config.acknowledgements.as_ref().ok_or(SaveError::Pending)?;
                        if saved.iter().any(|ack| !records.records().contains(ack)) {
                            return Err(SaveError::Pending);
                        }
                        let mut transitions = Vec::new();
                        for transition in config.transitions.take().unwrap_or_default() {
                            if saved.iter().any(|ack| {
                                ack.kind == AckKind::Updated
                                    && ack.owner == transition.owner
                                    && ack.generation == transition.generation
                            }) {
                                config.activations.push(transition.new);
                            } else {
                                transitions.push(transition);
                            }
                        }
                        config.transitions = Some(transitions);
                        config.withdrawals.retain(|intent| {
                            !saved.iter().any(|ack| {
                                ack.kind == AckKind::Withdrawn
                                    && ack.owner == intent.owner
                                    && intent
                                        .binding
                                        .configured()
                                        .is_some_and(|b| b.generation == ack.generation)
                            })
                        });
                        config.acknowledgements = None;
                    }
                    AckAction::None => unreachable!(),
                }
                candidate.tuning = encode_config(*config)?;
                Ok(())
            })();
            if let Err(error) = checked {
                if let Some(reservation) = &mut reservation {
                    let _ = reservation.publication_uncertain();
                }
                self.recovering = true;
                return Err(error);
            }
        }
        let saved = match self.persistence.save(&mut candidate) {
            Ok(saved) => saved,
            Err(error) => {
                if let Some(reservation) = &mut reservation {
                    let _ = reservation.publication_uncertain();
                    self.recovering = true;
                }
                return Err(error);
            }
        };
        let installed = match (&mut reservation, saved) {
            (Some(reservation), SavedState::Unavailable) => {
                let _ = reservation.publication_uncertain();
                false
            }
            #[cfg(windows)]
            (Some(reservation), SavedState::Native(proof)) => {
                reservation.install_native(*proof, WAIT).is_ok()
            }
            #[cfg(test)]
            (Some(reservation), SavedState::Fixture(bytes)) => reservation
                .install_fixture(&bytes, ConfigurationOrigin::fixture(), WAIT)
                .is_ok(),
            (None, _) => true,
        };
        self.state = candidate;
        self.rules = rules;
        if !installed {
            self.recovering = true;
            return Err(SaveError::Recovering);
        }
        Ok(())
    }
    fn configured(&self) -> Result<TuningConfig, SaveError> {
        match self.state.tuning.decode() {
            TuningLoad::Known(config) => Ok(*config),
            _ => Err(SaveError::Invalid),
        }
    }
    fn save_config(&mut self, config: TuningConfig) -> Result<(), SaveError> {
        let mut candidate = self.state.clone();
        candidate.tuning = encode_config(config)?;
        self.publish(candidate)
    }
    #[cfg(test)]
    fn attach_fixture(&mut self, runner: Runner) {
        self.runner = Some(runner);
    }
    pub(crate) fn activate_manual(
        &mut self,
        profile: ProfileId,
        feature: FeatureId,
        target: TargetIdentity,
    ) -> Result<OwnerId, SaveError> {
        let mut config = self.configured()?;
        if config
            .activations
            .iter()
            .chain(config.withdrawals.iter())
            .chain(config.transitions.iter().flatten().map(|t| &t.new))
            .any(|i| {
                i.binding.configured().is_some_and(|b| b.profile == profile)
                    && i.config.feature == feature
            })
        {
            return Err(SaveError::Pending);
        }
        let preference = config
            .profiles
            .iter_mut()
            .find(|p| p.id == profile)
            .and_then(|p| p.modules.iter_mut().find(|m| m.feature == feature))
            .ok_or(SaveError::Invalid)?;
        if preference.mode != ActivationMode::Manual {
            return Err(SaveError::Invalid);
        }
        preference.enabled = true;
        let owner = OwnerId::new(take_id(&mut config.allocations.next_owner)?)
            .map_err(|_| SaveError::Invalid)?;
        let generation =
            ConfigGeneration::new(take_id(&mut config.allocations.next_config_generation)?)
                .map_err(|_| SaveError::Invalid)?;
        config.activations.push(SavedIntent {
            version: 1,
            owner,
            binding: ActivationBinding::Manual(ConfiguredIntent {
                profile,
                generation,
            }),
            config: preference.config.clone(),
            target,
            mode: preference.mode,
        });
        self.save_config(config)?;
        Ok(owner)
    }
    pub(crate) fn edit_profile(
        &mut self,
        profile: ProfileId,
        new_config: ModuleEnvelope,
    ) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        if config.transitions.iter().flatten().any(|t| {
            t.new
                .binding
                .configured()
                .is_some_and(|b| b.profile == profile)
        }) {
            return Err(SaveError::Pending);
        }
        let preference = config
            .profiles
            .iter_mut()
            .find(|p| p.id == profile)
            .and_then(|p| {
                p.modules
                    .iter_mut()
                    .find(|m| m.feature == new_config.feature)
            })
            .ok_or(SaveError::Invalid)?;
        preference.config = new_config.clone();
        config.version = 2;
        config.transitions.get_or_insert_with(Vec::new);
        let mut remaining = Vec::new();
        for old in config.activations.drain(..) {
            if old
                .binding
                .configured()
                .is_some_and(|b| b.profile == profile)
                && old.config.feature == new_config.feature
            {
                let generation =
                    ConfigGeneration::new(take_id(&mut config.allocations.next_config_generation)?)
                        .map_err(|_| SaveError::Invalid)?;
                let mut new = old.clone();
                match &mut new.binding {
                    ActivationBinding::Manual(binding) => binding.generation = generation,
                    ActivationBinding::Game(game) => {
                        *game = GameActivation::new(
                            ConfiguredIntent {
                                profile: game.configured().profile,
                                generation,
                            },
                            game.rule().clone(),
                            game.eligibility(),
                            game.session().clone(),
                            game.processes().to_vec(),
                        )
                        .map_err(|_| SaveError::Invalid)?;
                    }
                    ActivationBinding::Unknown => return Err(SaveError::Invalid),
                }
                new.config = new_config.clone();
                config.transitions.as_mut().unwrap().push(SavedTransition {
                    version: 1,
                    owner: old.owner,
                    generation,
                    old,
                    new,
                });
            } else {
                remaining.push(old);
            }
        }
        config.activations = remaining;
        self.save_config(config)
    }
    pub(crate) fn disable_profile(&mut self, profile: ProfileId) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        if config.transitions.iter().flatten().any(|t| {
            t.new
                .binding
                .configured()
                .is_some_and(|b| b.profile == profile)
        }) {
            return Err(SaveError::Pending);
        }
        let found = config
            .profiles
            .iter_mut()
            .find(|p| p.id == profile)
            .ok_or(SaveError::Invalid)?;
        for m in &mut found.modules {
            m.enabled = false;
        }
        let mut remaining = Vec::new();
        for old in config.activations.drain(..) {
            if old
                .binding
                .configured()
                .is_some_and(|b| b.profile == profile)
            {
                config.withdrawals.push(old);
            } else {
                remaining.push(old);
            }
        }
        config.activations = remaining;
        self.save_config(config)
    }
    pub(crate) fn remove_profile(&mut self, profile: ProfileId) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        withdraw(&mut config, |i| {
            i.binding.configured().is_some_and(|b| b.profile == profile)
        })?;
        let before = config.profiles.len();
        config.profiles.retain(|p| p.id != profile);
        if config.profiles.len() == before {
            return Err(SaveError::Invalid);
        }
        config.associations.retain(|a| a.profile != profile);
        self.save_config(config)
    }
    pub(crate) fn remove_rule(&mut self, group: &GroupId, rule: &RuleId) -> Result<(), SaveError> {
        let group_index = self
            .rules
            .group_index_for_id(group)
            .ok_or(SaveError::Invalid)?;
        let rule_index = self
            .rules
            .rule_index_for_id(group_index, rule)
            .ok_or(SaveError::Invalid)?;
        let mut config = self.configured()?;
        withdraw(
            &mut config,
            |i| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().group()==group&&game.rule().rule()==rule),
        )?;
        config
            .associations
            .retain(|a| &a.group != group || &a.rule != rule);
        let mut candidate = self.state.clone();
        candidate.groups[group_index].programs.remove(rule_index);
        candidate.tuning = encode_config(config)?;
        let mut rules = self.rules.clone();
        rules.remove_rule(group_index, rule_index);
        self.publish_with_rules(candidate, rules, AckAction::None)
    }
    pub(crate) fn remove_group(&mut self, group: &GroupId) -> Result<(), SaveError> {
        let index = self
            .rules
            .group_index_for_id(group)
            .ok_or(SaveError::Invalid)?;
        let mut config = self.configured()?;
        withdraw(
            &mut config,
            |i| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().group()==group),
        )?;
        config.associations.retain(|a| &a.group != group);
        let mut candidate = self.state.clone();
        candidate.groups.remove(index);
        candidate.tuning = encode_config(config)?;
        let mut rules = self.rules.clone();
        rules.remove_group(index);
        self.publish_with_rules(candidate, rules, AckAction::None)
    }
    pub(crate) fn move_rule(
        &mut self,
        group: &GroupId,
        rule: &RuleId,
        destination: &GroupId,
    ) -> Result<(), SaveError> {
        if group == destination {
            return Ok(());
        }
        let source = self
            .rules
            .group_index_for_id(group)
            .ok_or(SaveError::Invalid)?;
        let index = self
            .rules
            .rule_index_for_id(source, rule)
            .ok_or(SaveError::Invalid)?;
        let target = self
            .rules
            .group_index_for_id(destination)
            .ok_or(SaveError::Invalid)?;
        let mut config = self.configured()?;
        withdraw(
            &mut config,
            |i| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().group()==group&&game.rule().rule()==rule),
        )?;
        config
            .associations
            .retain(|a| &a.group != group || &a.rule != rule);
        let mut candidate = self.state.clone();
        let app = candidate.groups[source].programs.remove(index);
        let insert = candidate.groups[target].programs.len();
        candidate.groups[target].programs.push(app);
        candidate.tuning = encode_config(config)?;
        let mut rules = self.rules.clone();
        rules
            .move_rule_between_groups_at(source, index, target, insert)
            .ok_or(SaveError::Invalid)?;
        self.publish_with_rules(candidate, rules, AckAction::None)
    }
    pub(crate) fn bind_rule(
        &mut self,
        group: GroupId,
        rule: RuleId,
        profile: ProfileId,
        eligibility: ActivationEligibility,
    ) -> Result<(), SaveError> {
        let index = self
            .rules
            .group_index_for_id(&group)
            .ok_or(SaveError::Invalid)?;
        self.rules
            .rule_index_for_id(index, &rule)
            .ok_or(SaveError::Invalid)?;
        let mut config = self.configured()?;
        if !config.profiles.iter().any(|p| p.id == profile) {
            return Err(SaveError::Invalid);
        }
        let old_rule = |i: &SavedIntent| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().rule()==&rule);
        if config.withdrawals.iter().any(old_rule)
            || config
                .transitions
                .iter()
                .flatten()
                .any(|t| old_rule(&t.old))
        {
            return Err(SaveError::Pending);
        }
        if config.associations.iter().any(|a| {
            a.group == group
                && a.rule == rule
                && a.profile == profile
                && a.eligibility == eligibility
        }) {
            return Ok(());
        }
        if config.activations.iter().any(old_rule) {
            withdraw(&mut config, old_rule)?;
            config.associations.retain(|a| a.rule != rule);
            self.save_config(config)?;
            return Err(SaveError::Pending);
        }
        config.associations.retain(|a| a.rule != rule);
        config.associations.push(Association {
            version: 1,
            group,
            rule,
            profile,
            eligibility,
        });
        self.save_config(config)
    }
    pub(crate) fn activate_game(
        &mut self,
        game: GameActivation,
        feature: FeatureId,
        target: TargetIdentity,
    ) -> Result<OwnerId, SaveError> {
        let mut config = self.configured()?;
        let configured = game.configured();
        if configured.generation.get()!=config.allocations.next_config_generation||config.activations.iter().chain(&config.withdrawals).chain(config.transitions.iter().flatten().map(|t|&t.new)).any(|i|matches!(&i.binding,ActivationBinding::Game(old) if old.rule().rule()==game.rule().rule())&&i.config.feature==feature){return Err(SaveError::Pending);}
        let preference = config
            .profiles
            .iter_mut()
            .find(|p| p.id == configured.profile)
            .and_then(|p| p.modules.iter_mut().find(|m| m.feature == feature))
            .ok_or(SaveError::Invalid)?;
        if preference.mode != ActivationMode::Session {
            return Err(SaveError::Invalid);
        }
        preference.enabled = true;
        let owner = OwnerId::new(take_id(&mut config.allocations.next_owner)?)
            .map_err(|_| SaveError::Invalid)?;
        take_id(&mut config.allocations.next_config_generation)?;
        let intent = SavedIntent {
            version: 1,
            owner,
            binding: ActivationBinding::Game(game),
            config: preference.config.clone(),
            target,
            mode: preference.mode,
        };
        if !config.request_matches(&intent) {
            return Err(SaveError::Invalid);
        }
        config.activations.push(intent);
        self.save_config(config)?;
        Ok(owner)
    }
    pub(crate) fn reconcile(&mut self) -> Result<(), SaveError> {
        self.tickets.retain(|_,ticket|!matches!(ticket.result(),Some(ref result) if matches!(result.outcome,super::platform::runner::TerminalOutcome::Completed(ref command) if command.error.is_none())));
        let config = self.configured()?;
        let runner = self.runner.as_ref().ok_or(SaveError::Pending)?;
        for intent in config
            .activations
            .iter()
            .chain(config.transitions.iter().flatten().map(|t| &t.new))
        {
            let generation = intent
                .binding
                .configured()
                .ok_or(SaveError::Invalid)?
                .generation
                .get();
            let key = (intent.owner, generation, false);
            if self.tickets.contains_key(&key) {
                continue;
            }
            if self.tickets.len() >= 256 {
                return Err(SaveError::Pending);
            }
            let ticket = runner
                .client()
                .submit(
                    Command::Acquire {
                        owner: intent.owner,
                        binding: intent.binding.clone(),
                        config: intent.config.clone(),
                        target: intent.target.clone(),
                        mode: intent.mode,
                    },
                    generation,
                )
                .map_err(|_| SaveError::Pending)?;
            self.tickets.insert(key, ticket);
        }
        for intent in &config.withdrawals {
            let generation = intent
                .binding
                .configured()
                .ok_or(SaveError::Invalid)?
                .generation
                .get();
            let key = (intent.owner, generation, true);
            if self.tickets.contains_key(&key) {
                continue;
            }
            if self.tickets.len() >= 256 {
                return Err(SaveError::Pending);
            }
            self.tickets.insert(
                key,
                runner
                    .client()
                    .submit(
                        Command::Release {
                            owner: intent.owner,
                        },
                        generation,
                    )
                    .map_err(|_| SaveError::Pending)?,
            );
        }
        Ok(())
    }
    pub(crate) fn wait(&mut self) -> Result<(), SaveError> {
        for ticket in self.tickets.values() {
            let result = ticket.wait(WAIT).ok_or(SaveError::Pending)?;
            if result.stale
                || !matches!(result.outcome,super::platform::runner::TerminalOutcome::Completed(ref result) if result.error.is_none())
            {
                return Err(SaveError::Recovering);
            }
        }
        self.tickets.clear();
        let config = self.configured()?;
        if !config.withdrawals.is_empty()
            || config.transitions.as_ref().is_some_and(|t| !t.is_empty())
        {
            self.publish_with_ack(self.state.clone(), AckAction::Record)?;
            self.publish_with_ack(self.state.clone(), AckAction::Prune)?;
            let installed = self.configured()?;
            let client = self.runner.as_ref().ok_or(SaveError::Pending)?.client();
            for withdrawn in config
                .withdrawals
                .iter()
                .filter(|withdrawn| !installed.withdrawals.contains(withdrawn))
            {
                let generation = withdrawn
                    .binding
                    .configured()
                    .ok_or(SaveError::Invalid)?
                    .generation
                    .get();
                if client
                    .retire_generation_owner(withdrawn.owner, generation)
                    .is_err()
                {
                    self.recovering = true;
                    return Err(SaveError::Recovering);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn shutdown(&mut self) -> Option<ShutdownResult> {
        self.runner.as_mut().map(|r| r.shutdown(WAIT))
    }
    pub(crate) fn snapshot(&self) -> Option<super::platform::runner::RunnerSnapshot> {
        self.runner.as_ref().map(Runner::snapshot)
    }
    pub(crate) fn catalog_check(
        catalog: &Catalog,
        config: &ModuleEnvelope,
    ) -> Result<(), SaveError> {
        catalog
            .validate_config(config)
            .map_err(|_| SaveError::Invalid)
    }
}
fn withdraw(
    config: &mut TuningConfig,
    selected: impl Fn(&SavedIntent) -> bool,
) -> Result<(), SaveError> {
    if config
        .transitions
        .iter()
        .flatten()
        .any(|t| selected(&t.old) || selected(&t.new))
    {
        return Err(SaveError::Pending);
    }
    let mut remaining = Vec::new();
    for old in config.activations.drain(..) {
        if selected(&old) {
            config.withdrawals.push(old);
        } else {
            remaining.push(old);
        }
    }
    config.activations = remaining;
    Ok(())
}
fn encode_config(config: TuningConfig) -> Result<TuningEnvelope, SaveError> {
    let raw = serde_json::to_string(&config).map_err(|_| SaveError::Invalid)?;
    if raw.len() > MAX_TUNING_CONFIG_BYTES {
        return Err(SaveError::Invalid);
    }
    let envelope: TuningEnvelope = serde_json::from_str(&raw).map_err(|_| SaveError::Invalid)?;
    if envelope.decode() != TuningLoad::Known(Box::new(config)) {
        return Err(SaveError::Invalid);
    }
    Ok(envelope)
}
fn take_id(counter: &mut u64) -> Result<u64, SaveError> {
    let value = *counter;
    *counter = counter
        .checked_add(1)
        .filter(|n| *n != u64::MAX)
        .ok_or(SaveError::Invalid)?;
    if value == 0 {
        return Err(SaveError::Invalid);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{features::tuning::Tuning, models::CoreGroup};
    use std::{
        fs::{File, OpenOptions},
        io::{Read, Write},
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    struct SavedFile {
        path: PathBuf,
        fail: Arc<AtomicBool>,
        after_ack: Option<Arc<dyn Fn() + Send + Sync>>,
    }
    impl StatePersistence for SavedFile {
        fn save(&mut self, state: &mut AppStateStorage) -> Result<SavedState, SaveError> {
            if self.fail.load(Ordering::Acquire) {
                return Err(SaveError::Storage);
            }
            let bytes = serde_json::to_vec(state).map_err(|_| SaveError::Storage)?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err(SaveError::Storage);
            }
            let mut file = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)
                .map_err(|_| SaveError::Storage)?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| SaveError::Storage)?;
            drop(file);
            if matches!(state.tuning.decode(),TuningLoad::Known(ref config) if config.acknowledgements.is_some())
            {
                if let Some(hook) = &self.after_ack {
                    hook();
                }
            }
            Ok(SavedState::Fixture(
                std::fs::read(&self.path).map_err(|_| SaveError::Storage)?,
            ))
        }
    }
    struct OwnedFile(PathBuf);
    impl Drop for OwnedFile {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_file(&self.0).unwrap();
            }
        }
    }
    fn fixture() -> (HeadlessTuning, OwnedFile, Arc<AtomicBool>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = PathBuf::from("target").join(format!(
            "p2b-headless-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let state = AppStateStorage::test_fixture(|_| {});
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&state).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let facade = Tuning::default().headless(
            state,
            Box::new(SavedFile {
                path: path.clone(),
                fail: fail.clone(),
                after_ack: None,
            }),
        );
        (facade, OwnedFile(path), fail)
    }
    fn saved(path: &PathBuf) -> AppStateStorage {
        let mut bytes = Vec::new();
        File::open(path)
            .unwrap()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(bytes.len() <= 16 * 1024 * 1024);
        serde_json::from_slice(&bytes).unwrap()
    }
    fn add_group(state: &mut AppStateStorage) -> Result<(), SaveError> {
        state.groups.push(CoreGroup {
            name: "Saved group".into(),
            cores: vec![0],
            programs: vec![],
            is_hidden: false,
            run_all_button: false,
        });
        Ok(())
    }
    fn configured_fixture(
        config: &ModuleEnvelope,
        catalog: Catalog,
    ) -> (HeadlessTuning, OwnedFile, OwnedFile, Arc<AtomicBool>) {
        configured_fixture_with_limits(config, catalog, Default::default())
    }
    fn configured_fixture_with_limits(
        config: &ModuleEnvelope,
        catalog: Catalog,
        limits: super::super::platform::runner::RunnerLimits,
    ) -> (HeadlessTuning, OwnedFile, OwnedFile, Arc<AtomicBool>) {
        let (mut facade, file, fail) = fixture();
        facade.state.tuning=serde_json::from_value(serde_json::json!({"version":2,"lineage":vec![1;32],"profiles":[{"version":1,"id":1,"name":"P","modules":[{"feature":config.feature,"enabled":false,"mode":"Manual","config":config}]}],"associations":[],"allocations":{"next_profile":2,"next_owner":1,"next_config_generation":1},"activations":[],"withdrawals":[],"transitions":[]})).unwrap();
        facade.persistence.save(&mut facade.state).unwrap();
        let journal = OwnedFile(file.0.with_extension("journal"));
        facade.attach_fixture(
            Runner::start_saved_fixture(limits, catalog, file.0.clone(), journal.0.clone())
                .unwrap(),
        );
        (facade, file, journal, fail)
    }
    #[test]
    fn p2b_headless_scalar_save_activate_edit_disable_reload_keeps_first_baseline() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, journal, _) = configured_fixture(&initial, catalog.clone());
        let profile = ProfileId::new(1).unwrap();
        let target = TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        });
        let owner = facade
            .activate_manual(profile, initial.feature.clone(), target)
            .unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 12);
        let baseline = facade.snapshot().unwrap().coordinator.resources[0]
            .baseline
            .clone();
        facade
            .edit_profile(
                profile,
                module.config_envelope(&ScalarConfig { value: 13 }).unwrap(),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 13);
        assert_eq!(
            facade.snapshot().unwrap().coordinator.resources[0].baseline,
            baseline
        );
        assert_eq!(
            facade.snapshot().unwrap().coordinator.owners[0].owner,
            owner
        );
        facade.disable_profile(profile).unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
        let reloaded = Runner::start_saved_fixture(
            super::super::platform::runner::RunnerLimits::default(),
            catalog,
            file.0.clone(),
            journal.0.clone(),
        )
        .unwrap();
        facade.attach_fixture(reloaded);
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_headless_post_ack_storage_failure_preserves_logical_state_and_closes_admission() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, _journal, fail) = configured_fixture(&initial, catalog);
        let before = serde_json::to_vec(&facade.state).unwrap();
        fail.store(true, Ordering::Release);
        assert_eq!(facade.save_base(add_group), Err(SaveError::Storage));
        assert_eq!(serde_json::to_vec(&facade.state).unwrap(), before);
        assert_eq!(serde_json::to_vec(&saved(&file.0)).unwrap(), before);
        fail.store(false, Ordering::Release);
        assert_eq!(facade.save_base(add_group), Err(SaveError::Recovering));
        assert_eq!(module.writes(), 0);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::TimedOut));
    }
    #[test]
    fn p2b_headless_terminal_ack_save_failure_retains_exact_update_before_prune() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, _journal, fail) = configured_fixture(&initial, catalog);
        let profile = ProfileId::new(1).unwrap();
        let target = TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        });
        facade
            .activate_manual(profile, initial.feature.clone(), target)
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        facade
            .edit_profile(
                profile,
                module.config_envelope(&ScalarConfig { value: 13 }).unwrap(),
            )
            .unwrap();
        facade.reconcile().unwrap();
        for ticket in facade.tickets.values() {
            assert!(ticket.wait(WAIT).is_some());
        }
        assert_eq!(module.value(), 13);
        let before = facade.configured().unwrap();
        fail.store(true, Ordering::Release);
        assert_eq!(facade.wait(), Err(SaveError::Storage));
        assert_eq!(facade.configured().unwrap(), before);
        assert_eq!(
            saved(&file.0).tuning.decode(),
            TuningLoad::Known(Box::new(before))
        );
        assert!(facade.configured().unwrap().acknowledgements.is_none());
        assert_eq!(facade.shutdown(), Some(ShutdownResult::TimedOut));
    }
    #[test]
    fn p2b_headless_user_setting_preserves_absence_and_restart_across_public_edit_reload() {
        use super::super::testing::user_setting::{UserConfig, UserModule};
        let module = UserModule::new(None, false);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module
            .config_envelope(&UserConfig {
                enabled: true,
                value: 12,
            })
            .unwrap();
        let (mut facade, file, journal, _) = configured_fixture(&initial, catalog.clone());
        let profile = ProfileId::new(1).unwrap();
        let session = SessionIdentity {
            user: UserIdentity {
                machine: MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                },
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(17).unwrap(),
            boot: BootId::new([2; 16]).unwrap(),
        };
        let owner = facade
            .activate_manual(
                profile,
                initial.feature.clone(),
                TargetIdentity::Session(session),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), Some(12));
        assert!(module.enabled());
        let baseline = facade
            .snapshot()
            .unwrap()
            .coordinator
            .resources
            .iter()
            .map(|r| r.baseline.clone())
            .collect::<Vec<_>>();
        facade
            .edit_profile(
                profile,
                module
                    .config_envelope(&UserConfig {
                        enabled: true,
                        value: 13,
                    })
                    .unwrap(),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), Some(13));
        assert_eq!(
            facade.snapshot().unwrap().coordinator.owners[0].owner,
            owner
        );
        assert_eq!(
            facade
                .snapshot()
                .unwrap()
                .coordinator
                .resources
                .iter()
                .map(|r| r.baseline.clone())
                .collect::<Vec<_>>(),
            baseline
        );
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
        facade = Tuning::default().headless(
            saved(&file.0),
            Box::new(SavedFile {
                path: file.0.clone(),
                fail: Arc::new(AtomicBool::new(false)),
                after_ack: None,
            }),
        );
        facade.attach_fixture(
            Runner::start_saved_fixture(
                super::super::platform::runner::RunnerLimits::default(),
                catalog,
                file.0.clone(),
                journal.0.clone(),
            )
            .unwrap(),
        );
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), Some(13));
        facade.disable_profile(profile).unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.native_value(), None);
        assert!(!module.enabled());
        assert_eq!(
            facade.snapshot().unwrap().coordinator.restoration_effect,
            Some(Effect::RestartRequired)
        );
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_headless_saved_intent_survives_queue_failure_without_id_reuse() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, _journal, _) = configured_fixture(&initial, catalog);
        let target = TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        });
        facade
            .activate_manual(
                ProfileId::new(1).unwrap(),
                initial.feature.clone(),
                target.clone(),
            )
            .unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let released = Arc::new(std::sync::Barrier::new(2));
        let first = Arc::new(AtomicBool::new(true));
        module.set_observe_hook({
            let entered = entered.clone();
            let released = released.clone();
            Arc::new(move || {
                if first.swap(false, Ordering::AcqRel) {
                    entered.wait();
                    released.wait();
                }
            })
        });
        let client = facade.runner.as_ref().unwrap().client();
        let observe = Command::Observe {
            feature: initial.feature.clone(),
            target,
        };
        let first_ticket = client.submit(observe.clone(), 1).unwrap();
        entered.wait();
        let queued = (0..16)
            .map(|_| client.submit(observe.clone(), 1).unwrap())
            .collect::<Vec<_>>();
        let result = facade.reconcile();
        released.wait();
        module.clear_observe_hook();
        assert_eq!(result, Err(SaveError::Pending));
        assert!(first_ticket.wait(WAIT).is_some());
        for ticket in queued {
            assert!(ticket.wait(WAIT).is_some());
        }
        let TuningLoad::Known(stored) = saved(&file.0).tuning.decode() else {
            panic!("saved intent");
        };
        assert_eq!(stored.activations.len(), 1);
        assert_eq!(stored.allocations.next_owner, 2);
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 12);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_headless_disable_before_acquire_commits_zero_write_ack() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, _journal, _) = configured_fixture(&initial, catalog);
        let profile = ProfileId::new(1).unwrap();
        facade
            .activate_manual(
                profile,
                initial.feature.clone(),
                TargetIdentity::Machine(MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }),
            )
            .unwrap();
        facade.disable_profile(profile).unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.writes(), 0);
        assert_eq!(module.reads(), 0);
        let TuningLoad::Known(stored) = saved(&file.0).tuning.decode() else {
            panic!("saved config");
        };
        assert!(stored.withdrawals.is_empty());
        assert!(stored.acknowledgements.is_none());
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    fn game(
        facade: &HeadlessTuning,
        profile: ProfileId,
        group: GroupId,
        rule: RuleId,
    ) -> GameActivation {
        let session = SessionIdentity {
            user: UserIdentity {
                machine: MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                },
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(17).unwrap(),
            boot: BootId::new([2; 16]).unwrap(),
        };
        let process = ProcessIdentity::new(
            session.clone(),
            101,
            123,
            ImagePath::new("C:\\fixture\\game.exe").unwrap(),
        )
        .unwrap();
        GameActivation::new(
            ConfiguredIntent {
                profile,
                generation: ConfigGeneration::new(
                    facade
                        .configured()
                        .unwrap()
                        .allocations
                        .next_config_generation,
                )
                .unwrap(),
            },
            RuleActivation::new(group, rule).unwrap(),
            ActivationEligibility::Running,
            session,
            vec![process],
        )
        .unwrap()
    }
    fn active_game_fixture() -> (
        HeadlessTuning,
        super::super::testing::scalar::ScalarModule,
        OwnedFile,
        OwnedFile,
    ) {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, journal, _) = configured_fixture(&initial, catalog);
        facade
            .save_base(|state| {
                add_group(state)?;
                add_group(state)?;
                state.groups[0]
                    .programs
                    .push(crate::app::models::AppToRun::new_installed(
                        "fixture".into(),
                        "fixture".into(),
                        os_api::PriorityClass::Normal,
                        false,
                    ));
                Ok(())
            })
            .unwrap();
        let group = facade.rules.group_id_for_index(0).unwrap();
        let rule = facade.rules.rule_id_for_index(0, 0).unwrap();
        let profile = ProfileId::new(1).unwrap();
        let mut config = facade.configured().unwrap();
        config.profiles[0].modules[0].mode = ActivationMode::Session;
        facade.save_config(config).unwrap();
        facade
            .bind_rule(
                group.clone(),
                rule.clone(),
                profile,
                ActivationEligibility::Running,
            )
            .unwrap();
        facade
            .activate_game(
                game(&facade, profile, group, rule),
                initial.feature,
                TargetIdentity::Machine(MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 12);
        (facade, module, file, journal)
    }
    #[test]
    fn p2b_review_base_save_refuses_bound_rule_delete_move_and_replacement() {
        for operation in 0..5 {
            let (mut facade, module, file, _journal) = active_game_fixture();
            if operation == 4 {
                facade
                    .save_base(|state| {
                        state.groups[1].programs = state.groups[0].programs.clone();
                        Ok(())
                    })
                    .unwrap();
            }
            let before = serde_json::to_vec(facade.state()).unwrap();
            let identities = facade.rules.to_persisted_identities();
            let result = facade.save_base(|state| {
                match operation {
                    0 | 4 => {
                        state.groups.remove(0);
                    }
                    1 => state.groups[0].programs.clear(),
                    2 => {
                        let rule = state.groups[0].programs.remove(0);
                        state.groups[1].programs.push(rule);
                    }
                    _ => state.groups.swap(0, 1),
                }
                Ok(())
            });
            assert_eq!(result, Err(SaveError::Invalid), "operation {operation}");
            assert_eq!(serde_json::to_vec(facade.state()).unwrap(), before);
            assert_eq!(serde_json::to_vec(&saved(&file.0)).unwrap(), before);
            assert_eq!(facade.rules.to_persisted_identities(), identities);
            facade
                .save_base(|state| {
                    state.theme_index = 1;
                    Ok(())
                })
                .unwrap();
            assert_eq!(module.value(), 12);
            facade.disable_profile(ProfileId::new(1).unwrap()).unwrap();
            facade.reconcile().unwrap();
            facade.wait().unwrap();
            assert_eq!(module.value(), 7);
            assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
        }
    }
    #[test]
    fn p2b_review_ack_prune_retires_owner_before_next_cycle_at_limit_one() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, _file, _journal, _) = configured_fixture_with_limits(
            &initial,
            catalog,
            super::super::platform::runner::RunnerLimits {
                generation_owners: 1,
                ..Default::default()
            },
        );
        let profile = ProfileId::new(1).unwrap();
        let mut previous = None;
        for _ in 0..2 {
            let owner = facade
                .activate_manual(
                    profile,
                    initial.feature.clone(),
                    TargetIdentity::Machine(MachineIdentity {
                        id: MachineId::new([1; 16]).unwrap(),
                    }),
                )
                .unwrap();
            assert!(previous.is_none_or(|old| owner > old));
            previous = Some(owner);
            facade.reconcile().unwrap();
            facade.wait().unwrap();
            assert_eq!(module.value(), 12);
            facade.disable_profile(profile).unwrap();
            facade.reconcile().unwrap();
            facade.wait().unwrap();
            assert_eq!(module.value(), 7);
            assert!(facade.configured().unwrap().withdrawals.is_empty());
        }
        assert_eq!(
            facade
                .runner
                .as_ref()
                .unwrap()
                .client()
                .submit(
                    Command::Release {
                        owner: previous.unwrap()
                    },
                    u64::MAX - 1
                )
                .err(),
            Some(super::super::platform::runner::SubmitError::RetiredOwner),
        );
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_review_partial_withdrawal_queue_failure_retires_only_pruned_owner() {
        use super::super::platform::runner::{RunnerLimits, SubmitError, TerminalOutcome};
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, _journal, _) = configured_fixture_with_limits(
            &initial,
            catalog,
            RunnerLimits {
                queue_capacity: 1,
                ..Default::default()
            },
        );
        let profile_a = ProfileId::new(1).unwrap();
        let profile_b = ProfileId::new(2).unwrap();
        let mut config = facade.configured().unwrap();
        let mut second = config.profiles[0].clone();
        second.id = profile_b;
        config.profiles.push(second);
        config.allocations.next_profile = 3;
        facade.save_config(config).unwrap();
        let target = TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        });
        let owner_a = facade
            .activate_manual(profile_a, initial.feature.clone(), target.clone())
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        let owner_b = facade
            .activate_manual(profile_b, initial.feature.clone(), target.clone())
            .unwrap();
        let intent_b = facade
            .configured()
            .unwrap()
            .activations
            .into_iter()
            .find(|intent| intent.owner == owner_b)
            .unwrap();
        let client = facade.runner.as_ref().unwrap().client();
        let shared = client
            .submit(
                Command::Acquire {
                    owner: owner_b,
                    binding: intent_b.binding.clone(),
                    config: intent_b.config.clone(),
                    target: intent_b.target.clone(),
                    mode: intent_b.mode,
                },
                intent_b.binding.configured().unwrap().generation.get(),
            )
            .unwrap();
        assert!(matches!(shared.wait(WAIT).unwrap().outcome,
            TerminalOutcome::Completed(ref result) if result.error.is_none()));
        assert_eq!(facade.snapshot().unwrap().coordinator.owners.len(), 2);
        assert_eq!(module.value(), 12);
        facade.disable_profile(profile_a).unwrap();
        facade.disable_profile(profile_b).unwrap();
        let pending_b = facade
            .configured()
            .unwrap()
            .withdrawals
            .into_iter()
            .find(|intent| intent.owner == owner_b)
            .unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let released = Arc::new(std::sync::Barrier::new(2));
        let first = Arc::new(AtomicBool::new(true));
        module.set_observe_hook({
            let entered = entered.clone();
            let released = released.clone();
            Arc::new(move || {
                if first.swap(false, Ordering::AcqRel) {
                    entered.wait();
                    released.wait();
                }
            })
        });
        let blocked = client
            .submit(
                Command::Observe {
                    feature: initial.feature,
                    target,
                },
                1,
            )
            .unwrap();
        entered.wait();
        let pressured = facade.reconcile();
        let queued = facade.snapshot().unwrap().queued.len();
        released.wait();
        module.clear_observe_hook();
        assert_eq!(pressured, Err(SaveError::Pending));
        assert_eq!(queued, 1); // Release A is queued behind the running Observe.
        assert!(blocked.wait(WAIT).is_some());
        assert_eq!(facade.wait(), Ok(()));
        assert_eq!(
            facade.configured().unwrap().withdrawals,
            vec![pending_b.clone()]
        );
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        assert_eq!(
            facade.snapshot().unwrap().coordinator.owners[0].owner,
            owner_b
        );
        assert_eq!(module.value(), 12);
        assert_eq!(
            client
                .submit(Command::Release { owner: owner_a }, u64::MAX - 1)
                .err(),
            Some(SubmitError::RetiredOwner)
        );
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
        assert!(facade.configured().unwrap().withdrawals.is_empty());
        assert_eq!(
            client
                .submit(Command::Release { owner: owner_b }, u64::MAX - 1)
                .err(),
            Some(SubmitError::RetiredOwner)
        );
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_review_game_edit_preserves_actor_owner_and_first_baseline() {
        use super::super::testing::scalar::ScalarConfig;
        let (mut facade, module, file, _journal) = active_game_fixture();
        let profile = ProfileId::new(1).unwrap();
        let old = facade.configured().unwrap().activations[0].clone();
        let baseline = facade.snapshot().unwrap().coordinator.resources[0]
            .baseline
            .clone();
        let next = module.config_envelope(&ScalarConfig { value: 13 }).unwrap();
        facade.edit_profile(profile, next.clone()).unwrap();
        let config = facade.configured().unwrap();
        let transition = &config.transitions.as_ref().unwrap()[0];
        assert_eq!(transition.old, old);
        assert_eq!(transition.new.owner, old.owner);
        assert_eq!(transition.new.target, old.target);
        assert_eq!(transition.new.mode, old.mode);
        assert_eq!(transition.new.config, next);
        let (ActivationBinding::Game(before), ActivationBinding::Game(after)) =
            (&old.binding, &transition.new.binding)
        else {
            panic!("Game actor retained")
        };
        assert_eq!(after.rule(), before.rule());
        assert_eq!(after.eligibility(), before.eligibility());
        assert_eq!(after.session(), before.session());
        assert_eq!(after.processes(), before.processes());
        assert_eq!(after.configured().profile, before.configured().profile);
        assert!(after.configured().generation > before.configured().generation);
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 13);
        assert_eq!(
            facade.snapshot().unwrap().coordinator.resources[0].baseline,
            baseline
        );
        assert_eq!(
            facade.snapshot().unwrap().coordinator.owners[0].owner,
            old.owner
        );
        facade.disable_profile(profile).unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_headless_delete_move_and_binding_transfer_save_exact_old_withdrawals() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        for operation in 0..5 {
            let module = ScalarModule::new(7);
            let mut catalog = Catalog::default();
            catalog.register(module.clone()).unwrap();
            let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
            let (mut facade, file, _journal, _) = configured_fixture(&initial, catalog);
            facade
                .save_base(|state| {
                    add_group(state)?;
                    add_group(state)?;
                    state.groups[0]
                        .programs
                        .push(crate::app::models::AppToRun::new_installed(
                            "fixture".into(),
                            "fixture".into(),
                            os_api::PriorityClass::Normal,
                            false,
                        ));
                    Ok(())
                })
                .unwrap();
            let group = facade.rules.group_id_for_index(0).unwrap();
            let destination = facade.rules.group_id_for_index(1).unwrap();
            let rule = facade.rules.rule_id_for_index(0, 0).unwrap();
            let profile = ProfileId::new(1).unwrap();
            let mut config = facade.configured().unwrap();
            config.profiles[0].modules[0].mode = ActivationMode::Session;
            config.profiles.push(Profile {
                version: 1,
                id: ProfileId::new(2).unwrap(),
                name: "new profile".into(),
                modules: config.profiles[0].modules.clone(),
            });
            config.allocations.next_profile = 3;
            facade.save_config(config).unwrap();
            facade
                .bind_rule(
                    group.clone(),
                    rule.clone(),
                    profile,
                    ActivationEligibility::Running,
                )
                .unwrap();
            let owner = facade
                .activate_game(
                    game(&facade, profile, group.clone(), rule.clone()),
                    initial.feature.clone(),
                    TargetIdentity::Machine(MachineIdentity {
                        id: MachineId::new([1; 16]).unwrap(),
                    }),
                )
                .unwrap();
            facade.reconcile().unwrap();
            facade.wait().unwrap();
            assert_eq!(module.value(), 12);
            let old = facade.configured().unwrap().activations[0].clone();
            match operation {
                0 => facade.remove_profile(profile).unwrap(),
                1 => facade.remove_rule(&group, &rule).unwrap(),
                2 => facade.remove_group(&group).unwrap(),
                3 => facade.move_rule(&group, &rule, &destination).unwrap(),
                _ => assert_eq!(
                    facade.bind_rule(
                        group.clone(),
                        rule.clone(),
                        ProfileId::new(2).unwrap(),
                        ActivationEligibility::Running
                    ),
                    Err(SaveError::Pending)
                ),
            }
            let pending = facade.configured().unwrap();
            assert_eq!(pending.withdrawals, vec![old]);
            assert_eq!(pending.withdrawals[0].owner, owner);
            assert_eq!(
                saved(&file.0).tuning.decode(),
                TuningLoad::Known(Box::new(pending))
            );
            assert_eq!(module.value(), 12);
            if operation == 3 {
                assert_eq!(facade.rules.rule_id_for_index(1, 0), Some(rule.clone()));
            }
            if operation >= 3 {
                assert_eq!(
                    facade.bind_rule(
                        if operation == 3 {
                            destination.clone()
                        } else {
                            group.clone()
                        },
                        rule.clone(),
                        ProfileId::new(2).unwrap(),
                        ActivationEligibility::Running
                    ),
                    Err(SaveError::Pending)
                );
            }
            facade.reconcile().unwrap();
            facade.wait().unwrap();
            assert_eq!(module.value(), 7);
            if operation >= 3 {
                let new_group = if operation == 3 { destination } else { group };
                let new_profile = ProfileId::new(2).unwrap();
                facade
                    .bind_rule(
                        new_group.clone(),
                        rule.clone(),
                        new_profile,
                        ActivationEligibility::Running,
                    )
                    .unwrap();
                let new_owner = facade
                    .activate_game(
                        game(&facade, new_profile, new_group, rule),
                        initial.feature.clone(),
                        TargetIdentity::Machine(MachineIdentity {
                            id: MachineId::new([1; 16]).unwrap(),
                        }),
                    )
                    .unwrap();
                assert!(new_owner > owner);
                facade.reconcile().unwrap();
                facade.wait().unwrap();
                assert_eq!(module.value(), 12);
            }
            assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
            assert_eq!(module.value(), 7);
        }
    }
    #[test]
    fn p2b_headless_repeated_reconcile_coalesces_same_owner_generation() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, _file, _journal, _) = configured_fixture(&initial, catalog);
        facade
            .activate_manual(
                ProfileId::new(1).unwrap(),
                initial.feature.clone(),
                TargetIdentity::Machine(MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }),
            )
            .unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let released = Arc::new(std::sync::Barrier::new(2));
        let first = Arc::new(AtomicBool::new(true));
        module.set_observe_hook({
            let entered = entered.clone();
            let released = released.clone();
            Arc::new(move || {
                if first.swap(false, Ordering::AcqRel) {
                    entered.wait();
                    released.wait();
                }
            })
        });
        facade.reconcile().unwrap();
        entered.wait();
        let results = (0..100).map(|_| facade.reconcile()).collect::<Vec<_>>();
        let queued = facade.snapshot().unwrap().queued.len();
        released.wait();
        module.clear_observe_hook();
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(queued, 0);
        assert_eq!(facade.tickets.len(), 1);
        facade.wait().unwrap();
        assert_eq!(module.writes(), 1);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p2b_headless_ack_observation_to_prune_race_retains_wire_and_pending_tuple() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, _journal, fail) = configured_fixture(&initial, catalog);
        let profile = ProfileId::new(1).unwrap();
        facade
            .activate_manual(
                profile,
                initial.feature.clone(),
                TargetIdentity::Machine(MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        facade
            .edit_profile(
                profile,
                module.config_envelope(&ScalarConfig { value: 13 }).unwrap(),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.persistence = Box::new(SavedFile {
            path: file.0.clone(),
            fail,
            after_ack: Some({
                let module = module.clone();
                Arc::new(move || module.external_change(99))
            }),
        });
        assert_eq!(facade.wait(), Err(SaveError::Recovering));
        let pending = facade.configured().unwrap();
        assert_eq!(pending.transitions.as_ref().unwrap().len(), 1);
        assert_eq!(pending.acknowledgements.as_ref().unwrap().len(), 1);
        assert_eq!(
            saved(&file.0).tuning.decode(),
            TuningLoad::Known(Box::new(pending))
        );
        assert_eq!(module.value(), 99);
        assert_eq!(module.writes(), 2);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::TimedOut));
    }
    #[test]
    fn p2b_headless_unsupported_saved_codec_retains_rejected_bytes_and_original_journal() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, journal, _) = configured_fixture(&initial, catalog);
        let profile = ProfileId::new(1).unwrap();
        facade
            .activate_manual(
                profile,
                initial.feature.clone(),
                TargetIdentity::Machine(MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        let original = std::fs::read(&journal.0).unwrap();
        let mut unsupported = module.config_envelope(&ScalarConfig { value: 13 }).unwrap();
        unsupported.version = SchemaVersion::new(2).unwrap();
        assert_eq!(
            facade.edit_profile(profile, unsupported),
            Err(SaveError::Recovering)
        );
        assert_eq!(std::fs::read(&journal.0).unwrap(), original);
        assert_eq!(module.value(), 12);
        assert_eq!(module.writes(), 1);
        let rejected = facade.configured().unwrap();
        assert_eq!(
            rejected.transitions.as_ref().unwrap()[0]
                .new
                .config
                .version
                .get(),
            2
        );
        assert_eq!(
            saved(&file.0).tuning.decode(),
            TuningLoad::Known(Box::new(rejected))
        );
        assert_eq!(facade.shutdown(), Some(ShutdownResult::TimedOut));
    }
    #[test]
    fn p2b_headless_reload_wire_ack_with_unknown_drift_does_not_prune_or_restore() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let initial = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let (mut facade, file, journal, fail) = configured_fixture(&initial, catalog.clone());
        let profile = ProfileId::new(1).unwrap();
        facade
            .activate_manual(
                profile,
                initial.feature.clone(),
                TargetIdentity::Machine(MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                }),
            )
            .unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        facade
            .edit_profile(
                profile,
                module.config_envelope(&ScalarConfig { value: 13 }).unwrap(),
            )
            .unwrap();
        facade.reconcile().unwrap();
        for ticket in facade.tickets.values() {
            assert!(ticket.wait(WAIT).is_some());
        }
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
        let mut wire = facade.configured().unwrap();
        let transition = &wire.transitions.as_ref().unwrap()[0];
        wire.acknowledgements = Some(vec![SavedAcknowledgement {
            version: 1,
            owner: transition.owner,
            generation: transition.generation,
            journal_revision: 1,
            kind: AckKind::Updated,
        }]);
        let mut state = saved(&file.0);
        state.tuning = encode_config(wire.clone()).unwrap();
        facade.persistence.save(&mut state).unwrap();
        let saved_before = std::fs::read(&file.0).unwrap();
        let journal_before = std::fs::read(&journal.0).unwrap();
        module.external_change(99);
        let writes = module.writes();
        facade = Tuning::default().headless(
            state,
            Box::new(SavedFile {
                path: file.0.clone(),
                fail,
                after_ack: None,
            }),
        );
        facade.attach_fixture(
            Runner::start_saved_fixture(
                super::super::platform::runner::RunnerLimits::default(),
                catalog,
                file.0.clone(),
                journal.0.clone(),
            )
            .unwrap(),
        );
        assert!(facade.wait().is_err());
        assert_eq!(facade.configured().unwrap(), wire);
        assert_eq!(std::fs::read(&file.0).unwrap(), saved_before);
        let original: super::super::platform::journal::JournalDocument =
            serde_json::from_slice(&journal_before).unwrap();
        let retained: super::super::platform::journal::JournalDocument =
            serde_json::from_slice(&std::fs::read(&journal.0).unwrap()).unwrap();
        assert_eq!(retained.origin, original.origin);
        assert_eq!(retained.sources, original.sources);
        assert_eq!(retained.owners[0].binding, original.owners[0].binding);
        assert_eq!(
            retained.resources[0].baseline,
            original.resources[0].baseline
        );
        assert_eq!(
            retained.resources[0].confirmed,
            original.resources[0].confirmed
        );
        assert!(matches!(
            retained.resources[0].status,
            ModuleStatus::RecoveryRequired | ModuleStatus::ExternalChange
        ));
        assert_eq!(module.value(), 99);
        assert_eq!(module.writes(), writes);
        assert!(matches!(
            facade.shutdown(),
            Some(ShutdownResult::RecoveryRequired | ShutdownResult::TimedOut)
        ));
    }
    #[test]
    fn p2b_headless_save_success_publishes_candidate_and_logical_ids() {
        let (mut facade, file, _) = fixture();
        facade.save_base(add_group).unwrap();
        let disk = saved(&file.0);
        assert_eq!(facade.state().groups.len(), 1);
        assert_eq!(disk.groups.len(), 1);
        assert_eq!(
            disk.rule_identities,
            Some(facade.rules().to_persisted_identities())
        );
        assert!(disk.rule_identities.unwrap().next_group_id > 0);
    }
    #[test]
    fn p2b_headless_save_failure_does_not_consume_ids_or_publish_candidate() {
        let (mut facade, file, fail) = fixture();
        let before = facade.rules().to_persisted_identities();
        fail.store(true, Ordering::Release);
        assert_eq!(facade.save_base(add_group), Err(SaveError::Storage));
        assert!(facade.state().groups.is_empty());
        assert!(saved(&file.0).groups.is_empty());
        assert_eq!(facade.rules().to_persisted_identities(), before);
        fail.store(false, Ordering::Release);
        facade.save_base(add_group).unwrap();
        assert_eq!(
            facade.rules().to_persisted_identities().next_group_id,
            before.next_group_id + 1
        );
    }
}
