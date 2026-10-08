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
pub(crate) trait StatePersistence: Send {
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
    saved_revision: u64,
    #[cfg(test)]
    fixture_paths: Option<(std::path::PathBuf, std::path::PathBuf)>,
    #[cfg(test)]
    fixture_targets: Vec<TargetIdentity>,
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
            saved_revision: 0,
            #[cfg(test)]
            fixture_paths: None,
            #[cfg(test)]
            fixture_targets: vec![],
        }
    }
    pub(crate) fn state(&self) -> &AppStateStorage {
        &self.state
    }
    pub(crate) fn create_profile(
        &mut self,
        name: String,
        modules: Vec<ModulePreference>,
    ) -> Result<ProfileId, SaveError> {
        if name.trim().is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || modules.len() > 64
        {
            return Err(SaveError::Invalid);
        }
        let mut config = match self.state.tuning.decode() {
            TuningLoad::Off => TuningConfig {
                version: 2,
                lineage: fresh_lineage()?,
                profiles: vec![],
                associations: vec![],
                allocations: Allocations {
                    next_profile: 1,
                    next_owner: 1,
                    next_config_generation: 1,
                },
                activations: vec![],
                withdrawals: vec![],
                transitions: Some(vec![]),
                acknowledgements: None,
            },
            TuningLoad::Known(config) => *config,
            _ => return Err(SaveError::Invalid),
        };
        if config.profiles.len() >= 64 {
            return Err(SaveError::Invalid);
        }
        let id = ProfileId::new(take_id(&mut config.allocations.next_profile)?)
            .map_err(|_| SaveError::Invalid)?;
        config.profiles.push(Profile {
            version: 1,
            id,
            name,
            modules,
        });
        self.save_config(config)?;
        Ok(id)
    }
    pub(crate) fn rules(&self) -> &RulesContext {
        &self.rules
    }
    pub(crate) fn catalog(&self) -> &Catalog {
        &self.catalog
    }
    pub(crate) fn saved_revision(&self) -> u64 {
        self.saved_revision
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn configure_fixture(
        &mut self,
        state: std::path::PathBuf,
        journal: std::path::PathBuf,
        targets: Vec<TargetIdentity>,
    ) {
        self.fixture_paths = Some((state, journal));
        self.fixture_targets = targets;
    }
    pub(in crate::app::features::tuning) fn start_saved_worker(&mut self) -> Result<(), SaveError> {
        if self.runner.is_some() {
            return Ok(());
        }
        #[cfg(test)]
        if let Some((state, journal)) = &self.fixture_paths {
            self.runner = Some(
                Runner::start_saved_fixture(
                    Default::default(),
                    self.catalog.clone(),
                    state.clone(),
                    journal.clone(),
                )
                .map_err(|_| SaveError::Pending)?,
            );
            return Ok(());
        }
        #[cfg(windows)]
        if self.state.native_configuration().is_some() {
            return self.start_worker();
        }
        Err(SaveError::Pending)
    }
    pub(in crate::app::features::tuning) fn available_targets(&self) -> Vec<TargetIdentity> {
        #[cfg(test)]
        if self.fixture_paths.is_some() {
            return self.fixture_targets.clone();
        }
        #[cfg(windows)]
        if self.catalog.descriptors().next().is_some()
            && self.state.native_configuration().is_some()
        {
            if let Ok(identity) = os_api::OS::tuning_machine_identity() {
                if let Ok(machine) =
                    super::platform::writer::native_machine(identity.source(), identity.bytes())
                {
                    return vec![TargetIdentity::Machine(machine)];
                }
            }
        }
        vec![]
    }
    pub(crate) fn set_module_preference(
        &mut self,
        profile: ProfileId,
        preference: ModulePreference,
        expected: Option<ModulePreference>,
    ) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        let descriptor = self
            .catalog
            .descriptors()
            .find(|d| d.id == preference.feature)
            .ok_or(SaveError::Invalid)?;
        if preference.feature != preference.config.feature
            || !descriptor.activation_modes.contains(&preference.mode)
            || self.catalog.validate_config(&preference.config).is_err()
        {
            return Err(SaveError::Invalid);
        }
        let selected = config
            .profiles
            .iter()
            .find(|p| p.id == profile)
            .ok_or(SaveError::Invalid)?;
        let old = selected
            .modules
            .iter()
            .find(|m| m.feature == preference.feature)
            .cloned();
        if old != expected || (old.is_none() && selected.modules.len() >= 64) {
            return Err(SaveError::Invalid);
        }
        if old.as_ref().is_some_and(|old| {
            old.enabled == preference.enabled
                && old.mode == preference.mode
                && old.config != preference.config
        }) {
            return self.edit_profile(profile, preference.config);
        }
        if old
            .as_ref()
            .is_some_and(|old| old.enabled && (!preference.enabled || old.mode != preference.mode))
        {
            withdraw(&mut config, |intent| {
                intent
                    .binding
                    .configured()
                    .is_some_and(|b| b.profile == profile)
                    && intent.config.feature == preference.feature
            })?;
        }
        let selected = config
            .profiles
            .iter_mut()
            .find(|p| p.id == profile)
            .unwrap();
        selected.modules.retain(|m| m.feature != preference.feature);
        selected.modules.push(preference);
        self.save_config(config)
    }
    pub(crate) fn edit_rule(
        &mut self,
        group: &GroupId,
        rule: &RuleId,
        app: crate::app::models::AppToRun,
    ) -> Result<(), SaveError> {
        let group_index = self
            .rules
            .group_index_for_id(group)
            .ok_or(SaveError::Invalid)?;
        let index = self
            .rules
            .rule_index_for_id(group_index, rule)
            .ok_or(SaveError::Invalid)?;
        let mut candidate = self.state.clone();
        if candidate.groups[group_index].programs[index] == app {
            return Ok(());
        }
        if let TuningLoad::Known(mut config) = candidate.tuning.decode() {
            withdraw(
                &mut config,
                |intent| matches!(&intent.binding, ActivationBinding::Game(game) if game.rule().group() == group && game.rule().rule() == rule),
            )?;
            candidate.tuning = encode_config(*config)?;
        } else if !candidate.tuning.is_off() {
            return Err(SaveError::Invalid);
        }
        candidate.groups[group_index].programs[index] = app;
        self.publish(candidate)
    }
    pub(crate) fn poll_shutdown(&mut self) -> Option<ShutdownResult> {
        self.runner.as_mut().map(|r| r.shutdown(Duration::ZERO))
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
        let rules = if candidate.rule_identities != self.state.rule_identities {
            let rules = RulesContext::from_storage(&candidate);
            let old = self.rules.to_persisted_identities();
            let next = rules.to_persisted_identities();
            if next.next_group_id < old.next_group_id
                || next.next_rule_id < old.next_rule_id
                || candidate.rule_identities.as_ref() != Some(&next)
            {
                return Err(SaveError::Invalid);
            }
            rules
        } else {
            let mut rules = self.rules.clone();
            rules.reconcile_with_storage(&candidate);
            rules
        };
        self.publish_with_rules(candidate, rules, AckAction::None)
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
        let revision = self
            .saved_revision
            .checked_add(1)
            .ok_or(SaveError::Invalid)?;
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
            (Some(reservation), SavedState::Fixture(bytes)) => {
                let origin = super::platform::configured_intent::saved_configuration(&bytes)
                    .map(|config| ConfigurationOrigin::fixture_lineage(config.lineage));
                origin
                    .is_some_and(|origin| reservation.install_fixture(&bytes, origin, WAIT).is_ok())
            }
            (None, _) => true,
        };
        self.state = candidate;
        self.rules = rules;
        self.saved_revision = revision;
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
    pub(in crate::app::features::tuning) fn attach_fixture(&mut self, runner: Runner) {
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
                        *game = game
                            .with_configured(ConfiguredIntent {
                                profile: game.configured().profile,
                                generation,
                            })
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
        let mut candidate = self.state.clone();
        if candidate.tuning.is_off() {
            candidate.groups[group_index].programs.remove(rule_index);
            let mut rules = self.rules.clone();
            rules.remove_rule(group_index, rule_index);
            return self.publish_with_rules(candidate, rules, AckAction::None);
        }
        let mut config = self.configured()?;
        withdraw(
            &mut config,
            |i| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().group()==group&&game.rule().rule()==rule),
        )?;
        config
            .associations
            .retain(|a| &a.group != group || &a.rule != rule);
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
        let mut candidate = self.state.clone();
        if candidate.tuning.is_off() {
            candidate.groups.remove(index);
            let mut rules = self.rules.clone();
            rules.remove_group(index);
            return self.publish_with_rules(candidate, rules, AckAction::None);
        }
        let mut config = self.configured()?;
        withdraw(
            &mut config,
            |i| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().group()==group),
        )?;
        config.associations.retain(|a| &a.group != group);
        candidate.groups.remove(index);
        candidate.tuning = encode_config(config)?;
        let mut rules = self.rules.clone();
        rules.remove_group(index);
        self.publish_with_rules(candidate, rules, AckAction::None)
    }
    #[cfg(test)]
    pub(crate) fn move_rule(
        &mut self,
        group: &GroupId,
        rule: &RuleId,
        destination: &GroupId,
    ) -> Result<(), SaveError> {
        let target = self
            .rules
            .group_index_for_id(destination)
            .ok_or(SaveError::Invalid)?;
        self.move_rule_at(
            group,
            rule,
            destination,
            self.state.groups[target].programs.len(),
        )
    }
    pub(crate) fn move_rule_at(
        &mut self,
        group: &GroupId,
        rule: &RuleId,
        destination: &GroupId,
        insert: usize,
    ) -> Result<(), SaveError> {
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
        let mut candidate = self.state.clone();
        if !self
            .rules
            .can_move_rule_between_groups_at(source, index, target, insert)
        {
            return Err(SaveError::Invalid);
        }
        if source == target && (insert == index || insert == index + 1) {
            return Ok(());
        }
        if source != target
            && candidate.groups[target]
                .programs
                .iter()
                .any(|app| app.get_key() == candidate.groups[source].programs[index].get_key())
        {
            return Err(SaveError::Invalid);
        }
        if source != target {
            if let TuningLoad::Known(mut config) = candidate.tuning.decode() {
                withdraw(
                    &mut config,
                    |i| matches!(&i.binding,ActivationBinding::Game(game) if game.rule().group()==group&&game.rule().rule()==rule),
                )?;
                config
                    .associations
                    .retain(|a| &a.group != group || &a.rule != rule);
                candidate.tuning = encode_config(*config)?;
            } else if !candidate.tuning.is_off() {
                return Err(SaveError::Invalid);
            }
        }
        super::super::rules::move_rule_between_groups_at(
            &mut candidate,
            source,
            index,
            target,
            insert,
        )
        .ok_or(SaveError::Invalid)?;
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
    pub(crate) fn unbind_rule(&mut self, group: &GroupId, rule: &RuleId) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        if !config
            .associations
            .iter()
            .any(|a| &a.group == group && &a.rule == rule)
        {
            return Err(SaveError::Invalid);
        }
        withdraw(
            &mut config,
            |intent| matches!(&intent.binding, ActivationBinding::Game(game) if game.rule().group() == group && game.rule().rule() == rule),
        )?;
        config
            .associations
            .retain(|a| &a.group != group || &a.rule != rule);
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
        if preference.mode != ActivationMode::Session
            && !(preference.mode == ActivationMode::ForegroundSession
                && game.eligibility() == ActivationEligibility::Foreground)
        {
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
    pub(in crate::app::features::tuning) fn withdraw_owner(
        &mut self,
        owner: OwnerId,
    ) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        if !config
            .activations
            .iter()
            .any(|intent| intent.owner == owner)
        {
            return Err(SaveError::Pending);
        }
        withdraw(&mut config, |intent| intent.owner == owner)?;
        self.save_config(config)
    }
    pub(crate) fn update_game_membership(
        &mut self,
        owner: OwnerId,
        game: GameActivation,
    ) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        if config
            .transitions
            .iter()
            .flatten()
            .any(|t| t.owner == owner)
            || config.withdrawals.iter().any(|i| i.owner == owner)
        {
            return Err(SaveError::Pending);
        }
        let index = config
            .activations
            .iter()
            .position(|i| i.owner == owner)
            .ok_or(SaveError::Invalid)?;
        let old = config.activations[index].clone();
        let ActivationBinding::Game(before) = &old.binding else {
            return Err(SaveError::Invalid);
        };
        if !game.membership_succeeds(before) {
            return Err(SaveError::Invalid);
        }
        let mut new = old.clone();
        new.binding = ActivationBinding::Game(game);
        if !config.request_matches(&new) {
            return Err(SaveError::Invalid);
        }
        config.activations.remove(index);
        config.version = 2;
        config
            .transitions
            .get_or_insert_with(Vec::new)
            .push(SavedTransition {
                version: 2,
                owner,
                generation: before.configured().generation,
                old,
                new,
            });
        self.save_config(config)
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
                    if config
                        .transitions
                        .iter()
                        .flatten()
                        .any(|t| t.owner == intent.owner && t.version == 2)
                    {
                        Command::UpdateMembership {
                            owner: intent.owner,
                            binding: intent.binding.clone(),
                        }
                    } else {
                        Command::Acquire {
                            owner: intent.owner,
                            binding: intent.binding.clone(),
                            config: intent.config.clone(),
                            target: intent.target.clone(),
                            mode: intent.mode,
                        }
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
    pub(in crate::app::features::tuning) fn drain_sessions(&mut self) -> Result<(), SaveError> {
        let TuningLoad::Known(config) = self.state.tuning.decode() else {
            return Ok(());
        };
        let session = |intent: &SavedIntent| matches!(intent.binding, ActivationBinding::Game(_));
        if !config.activations.iter().any(session)
            && !config.withdrawals.iter().any(session)
            && !config
                .transitions
                .iter()
                .flatten()
                .any(|transition| session(&transition.old) || session(&transition.new))
        {
            return Ok(());
        }
        self.reconcile()?;
        self.wait()?;
        let mut config = self.configured()?;
        if config.activations.iter().any(session) {
            withdraw(&mut config, session)?;
            self.save_config(config)?;
        }
        self.reconcile()?;
        self.wait()
    }
    pub(crate) fn snapshot(&self) -> Option<super::platform::runner::RunnerSnapshot> {
        self.runner.as_ref().map(Runner::snapshot)
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
fn fresh_lineage() -> Result<[u8; 32], SaveError> {
    let mut bytes = [0u8; 32];
    #[cfg(windows)]
    unsafe {
        windows::Win32::Security::Cryptography::BCryptGenRandom(
            None,
            &mut bytes,
            windows::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
        .ok()
        .map_err(|_| SaveError::Invalid)?;
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut bytes))
            .map_err(|_| SaveError::Invalid)?;
    }
    if bytes == [0; 32] {
        return Err(SaveError::Invalid);
    }
    Ok(bytes)
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
        facade.catalog = catalog.clone();
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
    fn p4_same_owner_membership_preserves_first_baseline_and_sources() {
        let (mut facade, module, file, _journal) = active_game_fixture();
        let old = facade.configured().unwrap().activations[0].clone();
        let ActivationBinding::Game(before) = &old.binding else {
            panic!("Game owner")
        };
        let mut processes = before.processes().to_vec();
        processes.push(
            ProcessIdentity::new(
                before.session().clone(),
                102,
                124,
                ImagePath::new("C:\\fixture\\descendant.exe").unwrap(),
            )
            .unwrap(),
        );
        let next = before.with_membership(processes).unwrap();
        let retained = facade.snapshot().unwrap().coordinator;
        let writes = module.writes();
        assert_eq!(
            facade.update_game_membership(old.owner, next.clone()),
            Ok(())
        );
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        let current = facade.snapshot().unwrap().coordinator;
        assert_eq!(current.owners.len(), 1);
        assert_eq!(current.owners[0].owner, old.owner);
        assert_eq!(current.owners[0].binding, ActivationBinding::Game(next));
        assert_eq!(current.owners[0].config, retained.owners[0].config);
        assert_eq!(current.resources, retained.resources);
        assert_eq!(module.value(), 12);
        assert_eq!(
            module.writes(),
            writes,
            "membership update performs no target writes"
        );
        facade.disable_profile(before.configured().profile).unwrap();
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
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
    fn p4_profile_bootstrap_is_explicit_saved_fresh_and_monotonic() {
        let (mut facade, file, fail) = fixture();
        assert_eq!(facade.state.tuning.decode(), TuningLoad::Off);
        let raw = std::fs::read(&file.0).unwrap();
        fail.store(true, Ordering::Release);
        assert_eq!(
            facade.create_profile("First".into(), vec![]),
            Err(SaveError::Storage)
        );
        assert_eq!(facade.state.tuning.decode(), TuningLoad::Off);
        assert_eq!(std::fs::read(&file.0).unwrap(), raw);
        fail.store(false, Ordering::Release);
        let first = facade.create_profile("First".into(), vec![]).unwrap();
        assert_eq!(first.get(), 1);
        let config = facade.configured().unwrap();
        assert_ne!(config.lineage, [0; 32]);
        assert_eq!(config.allocations.next_profile, 2);
        assert_eq!(config.allocations.next_owner, 1);
        assert_eq!(config.allocations.next_config_generation, 1);
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        facade.remove_profile(first).unwrap();
        let second = facade.create_profile("Next".into(), vec![]).unwrap();
        assert_eq!(second.get(), 2);
        assert_eq!(facade.configured().unwrap().lineage, config.lineage);
        let (mut other, _other_file, _) = fixture();
        other.create_profile("Other".into(), vec![]).unwrap();
        assert_ne!(other.configured().unwrap().lineage, config.lineage);
        assert_eq!(
            other.create_profile("".into(), vec![]),
            Err(SaveError::Invalid)
        );
    }
    #[test]
    fn p4_public_worker_saves_profile_off_caller_and_reports_failure() {
        use super::super::{TuningOperationState, TuningRequest, TuningRuntime};
        let (facade, file, fail) = fixture();
        let runtime = TuningRuntime::start(
            facade,
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        let id = runtime
            .request(TuningRequest::CreateProfile {
                name: "Saved".into(),
            })
            .unwrap();
        let until = std::time::Instant::now() + WAIT;
        while runtime.snapshot().revision == 0 && std::time::Instant::now() < until {
            std::thread::yield_now();
        }
        let first = runtime.snapshot();
        assert_eq!(first.profiles[0].name, "Saved");
        assert_eq!(saved(&file.0).tuning.decode(), first.state.tuning.decode());
        let raw = std::fs::read(&file.0).unwrap();
        fail.store(true, Ordering::Release);
        let failed = runtime
            .request(TuningRequest::CreateProfile {
                name: "Failed".into(),
            })
            .unwrap();
        let until = std::time::Instant::now() + WAIT;
        while matches!(
            runtime.snapshot().operation,
            TuningOperationState::Pending { .. }
        ) && std::time::Instant::now() < until
        {
            std::thread::yield_now();
        }
        let last = runtime.snapshot();
        assert!(failed > id);
        assert_eq!(
            last.operation,
            TuningOperationState::Refused {
                id: failed,
                error: SaveError::Storage
            }
        );
        assert_eq!(last.profiles.len(), 1);
        assert_eq!(last.revision, first.revision);
        assert_eq!(std::fs::read(&file.0).unwrap(), raw);
        runtime.begin_shutdown();
        let until = std::time::Instant::now() + WAIT;
        while !runtime.snapshot().shutdown_complete && std::time::Instant::now() < until {
            std::thread::yield_now();
        }
        assert!(runtime.snapshot().shutdown_complete);
    }
    #[test]
    fn p4_worker_panic_keeps_saved_state_and_closes_admission_with_fault_outcome() {
        use super::super::{TuningOperationState, TuningRequest, TuningRuntime};
        struct Panicking;
        impl StatePersistence for Panicking {
            fn save(&mut self, _: &mut AppStateStorage) -> Result<SavedState, SaveError> {
                panic!("fixture storage panic")
            }
        }
        let runtime = TuningRuntime::start(
            Tuning::default().headless(AppStateStorage::test_fixture(|_| {}), Box::new(Panicking)),
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        runtime
            .request(TuningRequest::CreateProfile {
                name: "Never saved".into(),
            })
            .unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(1);
        while matches!(
            runtime.snapshot().operation,
            TuningOperationState::Pending { .. }
        ) && std::time::Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            runtime.snapshot().operation,
            TuningOperationState::Recovering
        );
        assert_eq!(runtime.snapshot().revision, 0);
        assert!(runtime.snapshot().profiles.is_empty());
        assert_eq!(
            runtime.request(TuningRequest::CreateProfile {
                name: "Retry".into()
            }),
            Err(SaveError::Recovering)
        );
        runtime.begin_shutdown();
        let until = std::time::Instant::now() + Duration::from_secs(1);
        while runtime.snapshot().shutdown_outcome.is_none() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            runtime.snapshot().shutdown_outcome,
            Some(ShutdownResult::Faulted)
        );
    }
    #[test]
    fn p4_review_card_summary_and_resource_status_share_one_observation() {
        use super::super::session_reducer::{observe_cards, snapshot, TuningOperationState};
        let (mut facade, module, file, journal) = active_game_fixture();
        facade.configure_fixture(
            file.0.clone(),
            journal.0.clone(),
            vec![TargetIdentity::Machine(MachineIdentity {
                id: MachineId::new([1; 16]).unwrap(),
            })],
        );
        let before = facade.snapshot().unwrap().coordinator;
        let journal_before = std::fs::read(&journal.0).unwrap();
        let writes = module.writes();
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let read_count = reads.clone();
        let changing = module.clone();
        module.set_observe_hook(Arc::new(move || {
            if read_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                changing.external_change(99);
            }
        }));
        let observation = Arc::new(
            super::super::execution_bridge::ExecutionObservation::Unknown(
                super::super::execution_bridge::UnknownReason::Unavailable,
            ),
        );
        let mut cards = snapshot(
            &facade,
            facade.saved_revision(),
            TuningOperationState::Idle,
            observation.clone(),
            false,
        );
        observe_cards(&mut cards, &facade);
        module.clear_observe_hook();
        let card = &cards.profiles[0].modules[0];
        assert_eq!(card.observed.as_ref().unwrap().summary, "12");
        assert_eq!(
            card.status,
            Some(ModuleStatus::VerifiedActive(Effect::Immediate))
        );
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(module.writes(), writes);
        assert_eq!(facade.snapshot().unwrap().coordinator, before);
        assert_eq!(before.resources[0].baseline.bytes(), &7i32.to_le_bytes());
        assert_eq!(std::fs::read(&journal.0).unwrap(), journal_before);

        module.external_change(99);
        let mut cards = snapshot(
            &facade,
            facade.saved_revision(),
            TuningOperationState::Idle,
            observation,
            false,
        );
        observe_cards(&mut cards, &facade);
        assert_eq!(
            cards.profiles[0].modules[0]
                .observed
                .as_ref()
                .unwrap()
                .summary,
            "99"
        );
        assert_eq!(
            cards.profiles[0].modules[0].status,
            Some(ModuleStatus::ExternalChange)
        );
        assert_eq!(module.writes(), writes);
        assert_eq!(facade.snapshot().unwrap().coordinator, before);
        assert_eq!(std::fs::read(&journal.0).unwrap(), journal_before);
        module.external_change(12);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p4_review_fresh_card_observation_refuses_cached_verified_after_drift_or_denial() {
        use super::super::session_reducer::{observe_cards, snapshot, TuningOperationState};
        let (mut facade, module, file, journal) = active_game_fixture();
        let target = TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        });
        facade.configure_fixture(file.0.clone(), journal.0.clone(), vec![target]);
        let before = facade.snapshot().unwrap().coordinator;
        let journal_before = std::fs::read(&journal.0).unwrap();
        let writes = module.writes();
        assert_eq!(module.value(), 12);
        assert_eq!(before.resources[0].baseline.bytes(), &7i32.to_le_bytes());
        module.external_change(99);
        let observation = Arc::new(
            super::super::execution_bridge::ExecutionObservation::Unknown(
                super::super::execution_bridge::UnknownReason::Unavailable,
            ),
        );
        for (capability, status) in [
            (Capability::Available, ModuleStatus::ExternalChange),
            (
                Capability::Denied("fixture read refusal".into()),
                ModuleStatus::PermissionDenied,
            ),
            (
                Capability::Unsupported("fixture unavailable".into()),
                ModuleStatus::Unsupported,
            ),
        ] {
            module.set_capability(capability.clone());
            let mut cards = snapshot(
                &facade,
                facade.saved_revision(),
                TuningOperationState::Idle,
                observation.clone(),
                false,
            );
            observe_cards(&mut cards, &facade);
            let card = &cards.profiles[0].modules[0];
            assert_eq!(card.status, Some(status));
            if capability == Capability::Available {
                assert_eq!(card.observed.as_ref().unwrap().summary, "99");
            } else {
                assert!(card.observed.is_none());
                assert!(card.targets.is_empty());
            }
            assert_eq!(facade.snapshot().unwrap().coordinator, before);
            assert_eq!(std::fs::read(&journal.0).unwrap(), journal_before);
            assert_eq!(module.value(), 99);
            assert_eq!(module.writes(), writes);
        }
        // Exact evidence remains retained; read-only cards do not perform recovery.
        module.set_capability(Capability::Available);
        module.external_change(12);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p4_card_does_not_label_saved_edit_or_withdrawal_as_verified_current() {
        use super::super::session_reducer::{observe_cards, snapshot, TuningOperationState};
        let (mut facade, module, file, journal) = active_game_fixture();
        facade.configure_fixture(
            file.0.clone(),
            journal.0.clone(),
            vec![TargetIdentity::Machine(MachineIdentity {
                id: MachineId::new([1; 16]).unwrap(),
            })],
        );
        let profile = ProfileId::new(1).unwrap();
        facade
            .edit_profile(
                profile,
                module
                    .config_envelope(&super::super::testing::scalar::ScalarConfig { value: 13 })
                    .unwrap(),
            )
            .unwrap();
        let mut cards = snapshot(
            &facade,
            facade.saved_revision(),
            TuningOperationState::Idle,
            Arc::new(
                super::super::execution_bridge::ExecutionObservation::Unknown(
                    super::super::execution_bridge::UnknownReason::Unavailable,
                ),
            ),
            false,
        );
        observe_cards(&mut cards, &facade);
        assert_eq!(
            cards.profiles[0].modules[0].status,
            Some(ModuleStatus::Applying)
        );
        assert_eq!(module.value(), 12);
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        facade.disable_profile(profile).unwrap();
        let mut cards = snapshot(
            &facade,
            facade.saved_revision(),
            TuningOperationState::Idle,
            cards.observation.clone(),
            false,
        );
        observe_cards(&mut cards, &facade);
        assert_eq!(
            cards.profiles[0].modules[0].status,
            Some(ModuleStatus::Restoring)
        );
        assert!(!cards.profiles[0].modules[0].enabled);
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p4_session_worker_saves_membership_then_drains_only_session_intent() {
        use super::super::{
            execution_bridge::{ExecutionObservation, Member, ProcessFacts},
            session_reducer::SessionFacts,
            TuningRuntime,
        };
        let (facade, module, file, _journal) = active_game_fixture();
        let old = facade.configured().unwrap().activations[0].clone();
        let original = facade.snapshot().unwrap().coordinator.resources[0].clone();
        let ActivationBinding::Game(game) = old.binding.clone() else {
            panic!("Game")
        };
        let mut processes = game.processes().to_vec();
        processes.push(
            ProcessIdentity::new(
                game.session().clone(),
                102,
                124,
                ImagePath::new("C:\\fixture\\child.exe").unwrap(),
            )
            .unwrap(),
        );
        let facts = |p: &ProcessIdentity| ProcessFacts {
            pid: p.pid(),
            token: p.creation_token(),
            image: Arc::from(p.image_path.as_str()),
            #[cfg(windows)]
            native: None,
        };
        let member = Member {
            key: facade.state.groups[0].programs[0].get_key(),
            group: game.rule().group().clone(),
            rule: game.rule().rule().clone(),
            instances: processes.iter().map(facts).collect(),
        };
        let observation =
            ExecutionObservation::fixture(facade.state(), 2, vec![member], facts(&processes[0]));
        let runtime = TuningRuntime::start(
            facade,
            Arc::new(tokio::sync::RwLock::new(Default::default())),
        );
        let writes = module.writes();
        runtime.observe_session_fixture(
            observation,
            SessionFacts::Fixture {
                session: game.session().clone(),
                processes,
                targets: vec![old.target],
                exited: vec![],
            },
        );
        let until = std::time::Instant::now() + WAIT;
        while runtime
            .snapshot()
            .runner
            .as_ref()
            .unwrap()
            .coordinator
            .owners[0]
            .binding
            == old.binding
            && std::time::Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        let updated = runtime.snapshot();
        let ActivationBinding::Game(new_game) =
            &updated.runner.as_ref().unwrap().coordinator.owners[0].binding
        else {
            panic!("Game")
        };
        assert!(new_game.membership_succeeds(&game));
        assert_eq!(
            updated.runner.as_ref().unwrap().coordinator.resources[0].baseline,
            original.baseline
        );
        assert_eq!(
            updated.runner.as_ref().unwrap().coordinator.resources[0].baseline_snapshot,
            original.baseline_snapshot
        );
        assert_eq!(module.writes(), writes);
        assert_eq!(
            saved(&file.0).tuning.decode(),
            updated.state.tuning.decode()
        );
        runtime.begin_shutdown();
        let until = std::time::Instant::now() + WAIT;
        while runtime.snapshot().shutdown_outcome.is_none() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            runtime.snapshot().shutdown_outcome,
            Some(ShutdownResult::Complete)
        );
        let TuningLoad::Known(saved_config) = saved(&file.0).tuning.decode() else {
            panic!("Known")
        };
        assert!(
            saved_config.activations.is_empty()
                && saved_config.withdrawals.is_empty()
                && saved_config.transitions.unwrap().is_empty()
        );
        assert_eq!(module.value(), 7);
    }
    #[test]
    fn p4_public_profile_module_add_is_catalog_validated_and_stale_refused() {
        use super::super::testing::scalar::{ScalarConfig, ScalarModule};
        let (mut facade, file, _) = fixture();
        let module = ScalarModule::new(7);
        facade.catalog.register(module.clone()).unwrap();
        let profile = facade.create_profile("Public".into(), vec![]).unwrap();
        let preference = ModulePreference {
            feature: module
                .config_envelope(&ScalarConfig { value: 12 })
                .unwrap()
                .feature,
            enabled: true,
            mode: ActivationMode::Manual,
            config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
        };
        assert_eq!(
            facade.set_module_preference(profile, preference.clone(), None),
            Ok(())
        );
        assert_eq!(
            facade.configured().unwrap().profiles[0].modules,
            vec![preference.clone()]
        );
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        let raw = std::fs::read(&file.0).unwrap();
        let mut changed = preference.clone();
        changed.config = module.config_envelope(&ScalarConfig { value: 13 }).unwrap();
        assert_eq!(
            facade.set_module_preference(profile, changed.clone(), None),
            Err(SaveError::Invalid)
        );
        assert_eq!(std::fs::read(&file.0).unwrap(), raw);
        facade
            .set_module_preference(profile, changed, Some(preference))
            .unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(module.writes(), 0);
    }
    #[test]
    fn p4_public_unbind_saves_exact_withdrawal_without_disabling_profile() {
        let (mut facade, module, file, _journal) = active_game_fixture();
        let old = facade.configured().unwrap().activations[0].clone();
        let ActivationBinding::Game(game) = &old.binding else {
            panic!("Game")
        };
        let groups = serde_json::to_value(&facade.state.groups).unwrap();
        facade
            .unbind_rule(game.rule().group(), game.rule().rule())
            .unwrap();
        let saved_config = facade.configured().unwrap();
        assert!(saved_config.associations.is_empty());
        assert!(saved_config.activations.is_empty());
        assert_eq!(saved_config.withdrawals, vec![old]);
        assert!(saved_config.profiles[0].modules[0].enabled);
        assert_eq!(serde_json::to_value(&facade.state.groups).unwrap(), groups);
        assert_eq!(saved(&file.0).tuning.decode(), facade.state.tuning.decode());
        assert_eq!(module.value(), 12);
        facade.reconcile().unwrap();
        facade.wait().unwrap();
        assert_eq!(module.value(), 7);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
    }
    #[test]
    fn p4_membership_drift_retains_first_baseline_and_pending_saved_intent() {
        let (mut facade, module, file, _journal) = active_game_fixture();
        let before = facade.snapshot().unwrap().coordinator;
        let old = facade.configured().unwrap().activations[0].clone();
        let ActivationBinding::Game(game) = &old.binding else {
            panic!("Game")
        };
        let mut processes = game.processes().to_vec();
        processes.push(
            ProcessIdentity::new(
                game.session().clone(),
                102,
                124,
                ImagePath::new("C:\\fixture\\descendant.exe").unwrap(),
            )
            .unwrap(),
        );
        facade
            .update_game_membership(old.owner, game.with_membership(processes).unwrap())
            .unwrap();
        let saved_raw = std::fs::read(&file.0).unwrap();
        let writes = module.writes();
        module.external_change(99);
        facade.reconcile().unwrap();
        assert_eq!(facade.wait(), Err(SaveError::Recovering));
        assert_eq!(module.value(), 99);
        assert_eq!(module.writes(), writes);
        assert_eq!(std::fs::read(&file.0).unwrap(), saved_raw);
        let retained = facade.snapshot().unwrap().coordinator;
        assert_eq!(retained.owners[0].binding, old.binding);
        assert_eq!(retained.resources[0].baseline, before.resources[0].baseline);
        assert_eq!(
            retained.resources[0].baseline_snapshot,
            before.resources[0].baseline_snapshot
        );
        assert_eq!(
            retained.resources[0].current_snapshot,
            before.resources[0].current_snapshot
        );
        assert_eq!(
            retained.resources[0].confirmed,
            before.resources[0].confirmed
        );
        assert_eq!(facade.shutdown(), Some(ShutdownResult::RecoveryRequired));
    }
    #[test]
    fn p4_session_reducer_requires_complete_current_independent_actor_facts() {
        use super::super::{
            execution_bridge::{ExecutionObservation, Member, ProcessFacts, UnknownReason},
            session_reducer::{session_decision, SessionDecision, SessionFacts},
        };
        let (mut facade, module, _file, _journal) = active_game_fixture();
        let old = facade.configured().unwrap().activations[0].clone();
        let ActivationBinding::Game(game) = old.binding.clone() else {
            panic!("Game")
        };
        let mut processes = game.processes().to_vec();
        processes.push(
            ProcessIdentity::new(
                game.session().clone(),
                102,
                124,
                ImagePath::new("C:\\fixture\\child.exe").unwrap(),
            )
            .unwrap(),
        );
        let facts = |p: &ProcessIdentity| ProcessFacts {
            pid: p.pid(),
            token: p.creation_token(),
            image: Arc::from(p.image_path.as_str()),
            #[cfg(windows)]
            native: None,
        };
        let member = Member {
            key: facade.state.groups[0].programs[0].get_key(),
            group: game.rule().group().clone(),
            rule: game.rule().rule().clone(),
            instances: processes.iter().map(facts).collect(),
        };
        let observed = ExecutionObservation::fixture(
            &facade.state,
            2,
            vec![member.clone()],
            facts(&processes[0]),
        );
        let independent = SessionFacts::Fixture {
            session: game.session().clone(),
            processes: processes.clone(),
            targets: vec![old.target.clone()],
            exited: vec![],
        };
        assert!(observed.matches_state(&facade.state));
        assert!(facade.configured().unwrap().profiles[0].modules[0].enabled);
        assert!(facade.catalog.validate_config(&old.config).is_ok());
        assert!(game.with_membership(processes.clone()).is_ok());
        let mut foreground_state = facade.state.clone();
        let mut foreground_config = facade.configured().unwrap();
        foreground_config.profiles[0].modules[0].mode = ActivationMode::ForegroundSession;
        foreground_config.associations[0].eligibility = ActivationEligibility::Foreground;
        let foreground_game = GameActivation::new(
            game.configured(),
            game.rule().clone(),
            ActivationEligibility::Foreground,
            game.session().clone(),
            game.processes().to_vec(),
        )
        .unwrap();
        foreground_config.activations[0].binding = ActivationBinding::Game(foreground_game);
        foreground_config.activations[0].mode = ActivationMode::ForegroundSession;
        foreground_state.tuning = encode_config(foreground_config).unwrap();
        let mut descriptor = super::super::testing::scalar::descriptor();
        descriptor
            .activation_modes
            .push(ActivationMode::ForegroundSession);
        let mut foreground_catalog = Catalog::default();
        foreground_catalog
            .register(super::super::testing::scalar::ScalarModule::with_descriptor(descriptor))
            .unwrap();
        let other = ProcessFacts {
            pid: 999,
            token: 77,
            image: Arc::from("C:\\fixture\\other.exe"),
            #[cfg(windows)]
            native: None,
        };
        let foreground_lost = ExecutionObservation::fixture(
            &foreground_state,
            4,
            vec![Member {
                instances: vec![facts(&processes[0])],
                ..member.clone()
            }],
            other.clone(),
        );
        assert!(
            matches!(session_decision(&foreground_state, &foreground_lost, &independent, &foreground_catalog), Some(SessionDecision::Withdraw(owner)) if owner == old.owner)
        );
        let running_other = ExecutionObservation::fixture(
            &facade.state,
            4,
            vec![Member {
                instances: vec![facts(&processes[0])],
                ..member.clone()
            }],
            other,
        );
        assert!(
            session_decision(&facade.state, &running_other, &independent, &facade.catalog)
                .is_none(),
            "other focus does not release Running intent"
        );
        let Some(SessionDecision::Membership { owner, game: next }) =
            session_decision(&facade.state, &observed, &independent, &facade.catalog)
        else {
            panic!("same-owner churn must produce a saved membership update")
        };
        assert_eq!(owner, old.owner);
        assert!(next.membership_succeeds(&game));
        assert_eq!(next.configured(), game.configured());
        assert!(session_decision(
            &facade.state,
            &observed,
            &SessionFacts::Unavailable,
            &facade.catalog
        )
        .is_none());
        assert!(session_decision(
            &facade.state,
            &ExecutionObservation::Unknown(UnknownReason::Denied),
            &independent,
            &facade.catalog
        )
        .is_none());
        let absent = ExecutionObservation::fixture(&facade.state, 3, vec![], facts(&processes[0]));
        assert!(
            session_decision(&facade.state, &absent, &independent, &facade.catalog).is_none(),
            "missing membership is not positive expiry"
        );
        let writes = module.writes();
        facade.update_game_membership(owner, next).unwrap();
        assert!(
            session_decision(&facade.state, &observed, &independent, &facade.catalog).is_none(),
            "old config observation is stale"
        );
        assert_eq!(module.writes(), writes);
        assert_eq!(facade.shutdown(), Some(ShutdownResult::Complete));
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
