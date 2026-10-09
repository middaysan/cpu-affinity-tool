//! Candidate-state saves and the small field controller. Called only by the application worker.
use super::{
    config::*,
    persistence::{self, RecoveryStore},
    platform::{
        contract::*,
        controller::{Controller, OwnerKey, RecoveryIntent},
        registry::Registry,
    },
};
use crate::app::{
    features::rules::{self, RulesContext},
    models::{AppStateStorage, AppToRun},
    shared::ids::{GroupId, RuleId},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveError {
    Storage,
    Invalid,
    Pending,
    Recovering,
}
pub(crate) trait StatePersistence: Send {
    fn save(&mut self, candidate: &mut AppStateStorage) -> Result<SavedState, SaveError>;
    fn source_path(&self) -> Option<std::path::PathBuf> {
        None
    }
    #[cfg(test)]
    fn recovery_directory(&self) -> Option<std::path::PathBuf> {
        None
    }
}
pub(crate) enum SavedState {
    Unavailable,
}
pub(crate) struct SelectedStatePersistence;
impl StatePersistence for SelectedStatePersistence {
    fn save(&mut self, candidate: &mut AppStateStorage) -> Result<SavedState, SaveError> {
        candidate.try_save_state().map_err(|_| SaveError::Storage)?;
        Ok(SavedState::Unavailable)
    }
    fn source_path(&self) -> Option<std::path::PathBuf> {
        Some(AppStateStorage::selected_state_path())
    }
}
pub(crate) struct HeadlessTuning {
    state: AppStateStorage,
    rules: RulesContext,
    persistence: Box<dyn StatePersistence>,
    registry: Registry,
    pub(super) controller: Controller,
    revision: u64,
    pub(super) native_admitted: bool,
    pub(super) legacy_recovery: bool,
    pub(super) operation_error: Option<ModuleError>,
}
impl HeadlessTuning {
    pub(super) fn new(
        state: AppStateStorage,
        persistence: Box<dyn StatePersistence>,
        registry: Registry,
    ) -> Self {
        let rules = RulesContext::from_storage(&state);
        Self {
            state,
            rules,
            persistence,
            registry,
            controller: Controller::default(),
            revision: 0,
            native_admitted: false,
            legacy_recovery: false,
            operation_error: None,
        }
    }
    pub(crate) fn state(&self) -> &AppStateStorage {
        &self.state
    }
    pub(crate) fn rules(&self) -> &RulesContext {
        &self.rules
    }
    pub(crate) fn registry(&self) -> &Registry {
        &self.registry
    }
    pub(crate) fn saved_revision(&self) -> u64 {
        self.revision
    }
    #[cfg(test)]
    pub(crate) fn admit_test_modules(&mut self) {
        self.native_admitted = true;
    }
    pub(super) fn has_saved_source(&self) -> bool {
        self.persistence.source_path().is_some()
    }
    pub(super) fn native_failure(&mut self, error: ModuleError) -> SaveError {
        self.operation_error = Some(error);
        SaveError::Recovering
    }
    pub(super) fn verify_saved(&mut self) -> Result<(), SaveError> {
        let Some(path) = self.persistence.source_path() else {
            return if self.native_admitted {
                Ok(())
            } else {
                Err(SaveError::Recovering)
            };
        };
        let result = (|| {
            let saved = AppStateStorage::read_saved_for_tuning(&path)
                .map_err(|e| persistence::failure("saved configuration", e))?;
            if serde_json::to_vec(&saved)
                .map_err(|e| persistence::failure("saved configuration", e))?
                != serde_json::to_vec(&self.state)
                    .map_err(|e| persistence::failure("saved configuration", e))?
            {
                return Err(persistence::failure(
                    "saved configuration",
                    "Saved state changed outside this worker; reload before tuning",
                ));
            }
            if matches!(
                saved.tuning.decode(),
                TuningLoad::Unknown | TuningLoad::Blocked
            ) {
                return Err(persistence::failure(
                    "saved configuration",
                    "Saved tuning configuration is read-only or unavailable",
                ));
            }
            Ok(())
        })();
        result.map_err(|e| self.native_failure(e))
    }
    pub(crate) fn start_recovery(
        &mut self,
        store: Box<dyn RecoveryStore>,
    ) -> Result<(), SaveError> {
        if self.legacy_recovery {
            return Err(self.native_failure(persistence::failure(
                "legacy recovery",
                "Existing legacy recovery artifacts need attention",
            )));
        }
        self.verify_saved()?;
        let path = self
            .persistence
            .source_path()
            .ok_or(SaveError::Recovering)?;
        let source = persistence::source_identifier(&path).map_err(|e| self.native_failure(e))?;
        self.controller = Controller::restore(store, source, &self.registry)
            .map_err(|e| self.native_failure(e))?;
        self.native_admitted = true;
        self.reconcile_manual()?;
        self.operation_error = None;
        Ok(())
    }
    pub(super) fn start_selected_recovery(&mut self) {
        if self.controller.is_durable()
            || !self.has_saved_source()
            || self.registry.descriptors().next().is_none()
        {
            return;
        }
        #[cfg(test)]
        let directory = self
            .persistence
            .recovery_directory()
            .map_or_else(persistence::common_directory, Ok);
        #[cfg(not(test))]
        let directory = persistence::common_directory();
        let store =
            directory.and_then(|directory| persistence::DiskRecoveryStore::open(&directory));
        match store {
            Ok(store) => {
                let _ = self.start_recovery(Box::new(store));
            }
            Err(error) => {
                self.native_failure(error);
            }
        }
    }
    fn configured(&self) -> Result<TuningConfig, SaveError> {
        match self.state.tuning.decode() {
            TuningLoad::Off => Ok(TuningConfig::default()),
            TuningLoad::Known(c) => Ok(*c),
            _ => Err(SaveError::Invalid),
        }
    }
    fn publish(
        &mut self,
        mut candidate: AppStateStorage,
        mut rules: RulesContext,
    ) -> Result<(), SaveError> {
        let revision = self.revision.checked_add(1).ok_or(SaveError::Invalid)?;
        rules.reconcile_with_storage(&candidate);
        candidate.mark_ready_for_current_schema_save(rules.to_persisted_identities());
        self.persistence.save(&mut candidate)?;
        self.state = candidate;
        self.rules = rules;
        self.revision = revision;
        self.start_selected_recovery();
        Ok(())
    }
    fn save_config(&mut self, config: TuningConfig) -> Result<(), SaveError> {
        let mut candidate = self.state.clone();
        candidate.tuning = TuningEnvelope::from_config(&config).map_err(|_| SaveError::Invalid)?;
        self.publish(candidate, self.rules.clone())?;
        self.reconcile_manual()
    }
    pub(crate) fn create_profile(
        &mut self,
        name: String,
        modules: Vec<ModulePreference>,
    ) -> Result<ProfileId, SaveError> {
        let mut config = self.configured()?;
        if name.trim().is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || config.profiles.len() >= 64
        {
            return Err(SaveError::Invalid);
        }
        let id = ProfileId::new(config.next_profile).map_err(|_| SaveError::Invalid)?;
        config.next_profile = config
            .next_profile
            .checked_add(1)
            .ok_or(SaveError::Invalid)?;
        config.profiles.push(Profile {
            version: 1,
            id,
            name,
            modules,
        });
        self.save_config(config)?;
        Ok(id)
    }
    pub(crate) fn set_module_preference(
        &mut self,
        profile: ProfileId,
        preference: ModulePreference,
        expected: Option<ModulePreference>,
    ) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        let module = self
            .registry
            .get(&preference.feature)
            .ok_or(SaveError::Invalid)?;
        self.registry
            .validate_config(&preference.config)
            .map_err(|_| SaveError::Invalid)?;
        if !module
            .descriptor()
            .activation_modes
            .contains(&preference.mode)
        {
            return Err(SaveError::Invalid);
        }
        let selected = config
            .profiles
            .iter_mut()
            .find(|p| p.id == profile)
            .ok_or(SaveError::Invalid)?;
        let old = selected
            .modules
            .iter()
            .position(|m| m.feature == preference.feature);
        if old.map(|i| selected.modules[i].clone()) != expected {
            return Err(SaveError::Invalid);
        }
        let feature = preference.feature.clone();
        let manual = preference.mode == ActivationMode::Manual;
        if let Some(index) = old {
            selected.modules[index] = preference;
        } else {
            if selected.modules.len() >= 64 {
                return Err(SaveError::Invalid);
            }
            selected.modules.push(preference);
        }
        if !manual {
            config
                .manual_targets
                .retain(|t| t.profile != profile || t.feature != feature);
        }
        self.save_config(config)
    }
    pub(crate) fn disable_profile(&mut self, profile: ProfileId) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        let profile = config
            .profiles
            .iter_mut()
            .find(|p| p.id == profile)
            .ok_or(SaveError::Invalid)?;
        for module in &mut profile.modules {
            module.enabled = false;
        }
        self.save_config(config)
    }
    pub(crate) fn remove_profile(&mut self, profile: ProfileId) -> Result<(), SaveError> {
        let mut config = self.configured()?;
        if !config.profiles.iter().any(|p| p.id == profile) {
            return Err(SaveError::Invalid);
        }
        config.profiles.retain(|p| p.id != profile);
        config.associations.retain(|a| a.profile != profile);
        config.manual_targets.retain(|t| t.profile != profile);
        self.save_config(config)
    }
    pub(crate) fn bind_rule(
        &mut self,
        group: GroupId,
        rule: RuleId,
        profile: ProfileId,
        eligibility: ActivationEligibility,
    ) -> Result<(), SaveError> {
        self.rule_index(&group, &rule)?;
        let mut config = self.configured()?;
        if !config.profiles.iter().any(|p| p.id == profile) {
            return Err(SaveError::Invalid);
        }
        config
            .associations
            .retain(|a| a.group != group || a.rule != rule);
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
        config
            .associations
            .retain(|a| &a.group != group || &a.rule != rule);
        self.save_config(config)
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
        let next_rules = if candidate.rule_identities != self.state.rule_identities {
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
        if let TuningLoad::Known(config) = self.state.tuning.decode() {
            for association in &config.associations {
                let (g, r) = self.rule_index(&association.group, &association.rule)?;
                let ng = next_rules
                    .group_index_for_id(&association.group)
                    .ok_or(SaveError::Invalid)?;
                let nr = next_rules
                    .rule_index_for_id(ng, &association.rule)
                    .ok_or(SaveError::Invalid)?;
                if candidate.groups[ng].programs.get(nr) != self.state.groups[g].programs.get(r) {
                    return Err(SaveError::Invalid);
                }
            }
        }
        self.publish(candidate, next_rules)
    }
    fn rule_index(&self, group: &GroupId, rule: &RuleId) -> Result<(usize, usize), SaveError> {
        let g = self
            .rules
            .group_index_for_id(group)
            .ok_or(SaveError::Invalid)?;
        Ok((
            g,
            self.rules
                .rule_index_for_id(g, rule)
                .ok_or(SaveError::Invalid)?,
        ))
    }
    fn remove_associations(
        candidate: &mut AppStateStorage,
        remove: impl Fn(&Association) -> bool,
    ) -> Result<(), SaveError> {
        if candidate.tuning.is_off() {
            return Ok(());
        }
        let TuningLoad::Known(mut config) = candidate.tuning.decode() else {
            return Err(SaveError::Invalid);
        };
        config.associations.retain(|a| !remove(a));
        candidate.tuning = TuningEnvelope::from_config(&config).map_err(|_| SaveError::Invalid)?;
        Ok(())
    }
    pub(crate) fn edit_rule(
        &mut self,
        group: &GroupId,
        rule: &RuleId,
        app: AppToRun,
    ) -> Result<(), SaveError> {
        let (g, r) = self.rule_index(group, rule)?;
        let mut candidate = self.state.clone();
        candidate.groups[g].programs[r] = app;
        self.publish(candidate, self.rules.clone())
    }
    pub(crate) fn remove_rule(&mut self, group: &GroupId, rule: &RuleId) -> Result<(), SaveError> {
        let (g, r) = self.rule_index(group, rule)?;
        let mut candidate = self.state.clone();
        Self::remove_associations(&mut candidate, |a| &a.group == group && &a.rule == rule)?;
        candidate.groups[g].programs.remove(r);
        let mut rules = self.rules.clone();
        rules.remove_rule(g, r);
        self.publish(candidate, rules)
    }
    pub(crate) fn remove_group(&mut self, group: &GroupId) -> Result<(), SaveError> {
        let g = self
            .rules
            .group_index_for_id(group)
            .ok_or(SaveError::Invalid)?;
        let mut candidate = self.state.clone();
        Self::remove_associations(&mut candidate, |a| &a.group == group)?;
        candidate.groups.remove(g);
        let mut rules = self.rules.clone();
        rules.remove_group(g);
        self.publish(candidate, rules)
    }
    pub(crate) fn move_rule_at(
        &mut self,
        group: &GroupId,
        rule: &RuleId,
        destination: &GroupId,
        insert: usize,
    ) -> Result<(), SaveError> {
        let (source, index) = self.rule_index(group, rule)?;
        let target = self
            .rules
            .group_index_for_id(destination)
            .ok_or(SaveError::Invalid)?;
        if !self
            .rules
            .can_move_rule_between_groups_at(source, index, target, insert)
        {
            return Err(SaveError::Invalid);
        }
        if source == target && (insert == index || insert == index + 1) {
            return Ok(());
        }
        let mut candidate = self.state.clone();
        if source != target
            && candidate.groups[target]
                .programs
                .iter()
                .any(|app| app.get_key() == candidate.groups[source].programs[index].get_key())
        {
            return Err(SaveError::Invalid);
        }
        if source != target {
            Self::remove_associations(&mut candidate, |a| &a.group == group && &a.rule == rule)?;
        }
        rules::move_rule_between_groups_at(&mut candidate, source, index, target, insert)
            .ok_or(SaveError::Invalid)?;
        let mut rules = self.rules.clone();
        rules
            .move_rule_between_groups_at(source, index, target, insert)
            .ok_or(SaveError::Invalid)?;
        self.publish(candidate, rules)
    }
    pub(crate) fn activate_manual(
        &mut self,
        profile: ProfileId,
        feature: FeatureId,
        target: TargetIdentity,
    ) -> Result<(), SaveError> {
        if !self.native_admitted || self.legacy_recovery {
            return Err(
                self.native_failure(self.operation_error.clone().unwrap_or_else(|| {
                    persistence::failure("activate", "Durable native mutation admission is closed")
                })),
            );
        }
        self.verify_saved()?;
        if !self.controller.can_admit() {
            return Err(self.native_failure(
                self.controller
                    .error()
                    .unwrap_or_else(|| persistence::failure("activate", "Recovery required")),
            ));
        }
        let mut config = self.configured()?;
        let preference = config
            .profiles
            .iter()
            .find(|p| p.id == profile)
            .and_then(|p| p.modules.iter().find(|m| m.feature == feature))
            .filter(|m| m.enabled && m.mode == ActivationMode::Manual)
            .ok_or(SaveError::Invalid)?;
        let module = self
            .registry
            .get(&feature)
            .ok_or(SaveError::Invalid)?
            .clone();
        if !module.targets().contains(&target) {
            return Err(SaveError::Invalid);
        }
        let desired = preference.config.clone();
        config
            .manual_targets
            .retain(|t| t.profile != profile || t.feature != feature);
        config.manual_targets.push(ManualTarget {
            profile,
            feature: feature.clone(),
            target: target.clone(),
        });
        let mut candidate = self.state.clone();
        candidate.tuning = TuningEnvelope::from_config(&config).map_err(|_| SaveError::Invalid)?;
        self.publish(candidate, self.rules.clone())?;
        self.verify_saved()?;
        self.controller
            .acquire(
                OwnerKey::Manual { profile, feature },
                module,
                target,
                desired,
            )
            .map_err(|e| self.native_failure(e))
    }
    fn reconcile_manual(&mut self) -> Result<(), SaveError> {
        if !self.native_admitted || self.legacy_recovery {
            return Ok(());
        }
        self.verify_saved()?;
        let config = self.configured()?;
        let mut failed = false;
        for owner in self.controller.owners() {
            if owner.is_session() {
                continue;
            }
            let preference = config
                .profiles
                .iter()
                .find(|p| p.id == owner.profile())
                .and_then(|p| p.modules.iter().find(|m| &m.feature == owner.feature()))
                .filter(|m| m.enabled && m.mode == ActivationMode::Manual);
            if let Some(preference) = preference {
                let target = self.controller.target(&owner);
                let selected = config
                    .manual_targets
                    .iter()
                    .find(|t| t.profile == owner.profile() && &t.feature == owner.feature());
                if let (Some(target), Some(module), Some(selected)) = (
                    target,
                    self.registry.get(owner.feature()).cloned(),
                    selected,
                ) {
                    if !module.targets().contains(&target) {
                        let _ = self
                            .controller
                            .recover_owner(&owner, RecoveryIntent::Unknown);
                        failed = true;
                        continue;
                    }
                    let matching = selected.target == target
                        && self
                            .controller
                            .owner_record(&owner)
                            .is_some_and(|record| record.config == preference.config);
                    if matching {
                        if self.controller.needs_recovery(&owner) {
                            failed |= self
                                .controller
                                .recover_owner(&owner, RecoveryIntent::Keep)
                                .is_err();
                        }
                    } else {
                        failed |= self
                            .controller
                            .recover_owner(&owner, RecoveryIntent::Restore)
                            .is_err();
                        if !failed && self.controller.can_admit() {
                            failed |= self
                                .controller
                                .acquire(
                                    owner.clone(),
                                    module,
                                    selected.target.clone(),
                                    preference.config.clone(),
                                )
                                .is_err();
                        }
                    }
                } else {
                    let _ = self
                        .controller
                        .recover_owner(&owner, RecoveryIntent::Unknown);
                    failed = true;
                }
            } else {
                failed |= self
                    .controller
                    .recover_owner(&owner, RecoveryIntent::Restore)
                    .is_err();
            }
        }
        if failed {
            if let Some(error) = self.controller.error() {
                self.native_failure(error);
            }
            Err(SaveError::Recovering)
        } else {
            Ok(())
        }
    }
}
