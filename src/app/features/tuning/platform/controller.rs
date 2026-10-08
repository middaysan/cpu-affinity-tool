//! Own only declared fields; publish original values and every write/ownership boundary.
use super::{
    codec::ModuleEnvelope,
    contract::*,
    registry::{FieldChange, Module, Registry},
};
use crate::app::features::tuning::persistence::{
    self, ActivationFacts, FieldRecord, OwnerRecord, RecoveryDocument, RecoveryStore,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum OwnerKey {
    Manual {
        profile: ProfileId,
        feature: FeatureId,
    },
    Session {
        profile: ProfileId,
        feature: FeatureId,
        group: String,
        rule: String,
        pid: u32,
        token: u64,
    },
}
impl OwnerKey {
    pub fn feature(&self) -> &FeatureId {
        match self {
            Self::Manual { feature, .. } | Self::Session { feature, .. } => feature,
        }
    }
    pub fn profile(&self) -> ProfileId {
        match self {
            Self::Manual { profile, .. } | Self::Session { profile, .. } => *profile,
        }
    }
    pub fn is_session(&self) -> bool {
        matches!(self, Self::Session { .. })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifetimeEvidence {
    Live,
    Exited,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryIntent {
    Keep,
    Restore,
    Unknown,
}
#[derive(Clone)]
struct OwnedField {
    module: Arc<dyn Module>,
    baseline: Vec<u8>,
    written: Vec<u8>,
    desired: Vec<u8>,
    attempted: Option<Vec<u8>>,
    owners: BTreeSet<OwnerKey>,
}
#[cfg(test)]
#[derive(Clone, Debug)]
pub struct FieldSnapshot {
    pub key: ResourceKey,
    pub baseline: Vec<u8>,
    pub written: Vec<u8>,
    pub attempted: Option<Vec<u8>>,
    pub owners: Vec<OwnerKey>,
}
#[derive(Default)]
pub struct Controller {
    fields: BTreeMap<ResourceKey, OwnedField>,
    records: BTreeMap<OwnerKey, OwnerRecord>,
    errors: BTreeMap<OwnerKey, ModuleError>,
    store: Option<Box<dyn RecoveryStore>>,
    source: String,
    pending: BTreeSet<OwnerKey>,
    fault: Option<ModuleError>,
    pub restoration_effect: Option<Effect>,
}
impl Controller {
    pub(crate) fn restore(
        mut store: Box<dyn RecoveryStore>,
        source: String,
        registry: &Registry,
    ) -> Result<Self, ModuleError> {
        let document = store.load()?;
        let mut controller = Self {
            source,
            store: Some(store),
            ..Self::default()
        };
        if let Some(document) = document {
            document.validate()?;
            if document.source != controller.source {
                return Err(persistence::failure(
                    "state source",
                    "Recovery belongs to a different state.json location",
                ));
            }
            for record in document.owners {
                registry.validate_config(&record.config)?;
                controller.pending.insert(record.owner.clone());
                controller.records.insert(record.owner.clone(), record);
            }
            for field in document.fields {
                let module = registry
                    .get(&field.module)
                    .ok_or_else(|| {
                        persistence::failure("recovery module", "Recorded module unavailable")
                    })?
                    .clone();
                if module.descriptor().config_version != field.version
                    || module.descriptor().access != ModuleAccess::Managed
                    || !module
                        .descriptor()
                        .target_scopes
                        .contains(&field.key.target.scope())
                {
                    return Err(persistence::failure(
                        "recovery module",
                        "Recorded module version or target unavailable",
                    ));
                }
                controller.fields.insert(
                    field.key,
                    OwnedField {
                        module,
                        baseline: field.baseline,
                        written: field.current,
                        desired: field.desired,
                        attempted: field.attempted,
                        owners: field.owners.into_iter().collect(),
                    },
                );
            }
        }
        Ok(controller)
    }
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
    pub fn owners(&self) -> BTreeSet<OwnerKey> {
        self.fields
            .values()
            .flat_map(|f| f.owners.iter().cloned())
            .collect()
    }
    pub(crate) fn owner_record(&self, owner: &OwnerKey) -> Option<&OwnerRecord> {
        self.records.get(owner)
    }
    pub(crate) fn is_durable(&self) -> bool {
        self.store.is_some()
    }
    pub(crate) fn needs_recovery(&self, owner: &OwnerKey) -> bool {
        self.pending.contains(owner)
    }
    pub(crate) fn can_admit(&self) -> bool {
        self.fault.is_none()
            && self.pending.is_empty()
            && self.fields.values().all(|f| f.attempted.is_none())
    }
    pub(crate) fn error(&self) -> Option<ModuleError> {
        self.fault
            .clone()
            .or_else(|| self.errors.values().next().cloned())
            .or_else(|| {
                (!self.pending.is_empty()).then(|| {
                    persistence::failure(
                        "recovery intent",
                        "Owner intent or process lifetime is unknown",
                    )
                })
            })
    }
    #[cfg(test)]
    pub fn fields(&self) -> Vec<FieldSnapshot> {
        self.fields
            .iter()
            .map(|(key, f)| FieldSnapshot {
                key: key.clone(),
                baseline: f.baseline.clone(),
                written: f.written.clone(),
                attempted: f.attempted.clone(),
                owners: f.owners.iter().cloned().collect(),
            })
            .collect()
    }
    pub fn target(&self, owner: &OwnerKey) -> Option<TargetIdentity> {
        self.fields
            .iter()
            .find(|(_, f)| f.owners.contains(owner))
            .map(|(key, _)| key.target.clone())
    }
    pub fn status(&self, owner: &OwnerKey) -> Option<ModuleStatus> {
        if let Some(e) = self.errors.get(owner) {
            return Some(match e.kind {
                ErrorKind::ExternalChange => ModuleStatus::ExternalChange,
                ErrorKind::Denied => ModuleStatus::PermissionDenied,
                _ => ModuleStatus::PartialFailure,
            });
        }
        if self.pending.contains(owner) || self.fault.is_some() {
            return Some(ModuleStatus::RecoveryRequired);
        }
        self.fields
            .values()
            .find(|f| f.owners.contains(owner))
            .map(|f| ModuleStatus::VerifiedActive(f.module.descriptor().effect))
    }
    pub fn status_for(&self, profile: ProfileId, feature: &FeatureId) -> Option<ModuleStatus> {
        let matches = |owner: &&OwnerKey| owner.profile() == profile && owner.feature() == feature;
        if let Some(owner) = self.errors.keys().find(matches) {
            return self.status(owner);
        }
        self.owners()
            .iter()
            .find(matches)
            .and_then(|owner| self.status(owner))
    }
    fn document(&self) -> RecoveryDocument {
        RecoveryDocument {
            version: 1,
            source: self.source.clone(),
            fields: self
                .fields
                .iter()
                .map(|(key, f)| FieldRecord {
                    module: f.module.descriptor().id,
                    version: f.module.descriptor().config_version,
                    key: key.clone(),
                    baseline: f.baseline.clone(),
                    current: f.written.clone(),
                    desired: f.desired.clone(),
                    attempted: f.attempted.clone(),
                    owners: f.owners.iter().cloned().collect(),
                })
                .collect(),
            owners: self.records.values().cloned().collect(),
        }
    }
    fn persist(&mut self) -> Result<(), ModuleError> {
        if let Some(error) = &self.fault {
            return Err(error.clone());
        }
        if self.store.is_none() {
            return Ok(());
        }
        let document = self.document();
        let result = document
            .validate()
            .and_then(|_| self.store.as_mut().unwrap().publish(&document));
        if let Err(error) = &result {
            self.fault = Some(error.clone());
        }
        result
    }
    fn remember(&mut self, owner: &OwnerKey, result: &Result<(), ModuleError>) {
        match result {
            Ok(()) => {
                self.errors.remove(owner);
            }
            Err(e) => {
                if self.errors.len() < 256 || self.errors.contains_key(owner) {
                    self.errors.insert(owner.clone(), e.clone());
                }
            }
        }
    }
    pub fn inspect(&mut self) {
        let mut failed = BTreeMap::new();
        for (key, field) in &self.fields {
            let error = match field.module.read(key) {
                Ok(actual) if field.attempted.is_none() && actual == field.written => None,
                Ok(_) => Some(ModuleError::new(
                    ErrorKind::ExternalChange,
                    "observe owned field",
                )),
                Err(error) => Some(error),
            };
            if let Some(error) = error {
                for owner in &field.owners {
                    failed.insert(owner.clone(), error.clone());
                    if self.store.is_some() {
                        self.pending.insert(owner.clone());
                    }
                }
            }
        }
        self.errors.retain(|owner, error| {
            failed.contains_key(owner)
                || !matches!(
                    error.kind,
                    ErrorKind::ExternalChange | ErrorKind::Read | ErrorKind::Denied
                )
        });
        self.errors.extend(failed);
        if self.errors.len() > 256 {
            let owners = self.owners();
            self.errors.retain(|owner, _| owners.contains(owner));
        }
    }
    pub fn acquire(
        &mut self,
        owner: OwnerKey,
        module: Arc<dyn Module>,
        target: TargetIdentity,
        config: ModuleEnvelope,
    ) -> Result<(), ModuleError> {
        self.acquire_record(owner, module, target, config, None)
    }
    pub(crate) fn acquire_session(
        &mut self,
        owner: OwnerKey,
        module: Arc<dyn Module>,
        target: TargetIdentity,
        config: ModuleEnvelope,
        activation: ActivationFacts,
    ) -> Result<(), ModuleError> {
        self.acquire_record(owner, module, target, config, Some(activation))
    }
    fn acquire_record(
        &mut self,
        owner: OwnerKey,
        module: Arc<dyn Module>,
        target: TargetIdentity,
        config: ModuleEnvelope,
        activation: Option<ActivationFacts>,
    ) -> Result<(), ModuleError> {
        let result = self.acquire_inner(&owner, module, target, config, activation);
        if self.store.is_some() && result.is_err() && self.owners().contains(&owner) {
            self.pending.insert(owner.clone());
        }
        self.remember(&owner, &result);
        result
    }
    fn acquire_inner(
        &mut self,
        owner: &OwnerKey,
        module: Arc<dyn Module>,
        target: TargetIdentity,
        config: ModuleEnvelope,
        activation: Option<ActivationFacts>,
    ) -> Result<(), ModuleError> {
        let invalid = || ModuleError::new(ErrorKind::InvalidConfig, "fields");
        if !self.can_admit() {
            return Err(self.error().unwrap_or_else(|| {
                persistence::failure("acquire", "Unresolved recovery evidence")
            }));
        }
        let owners = self.owners();
        if owners.len() >= 256 && !owners.contains(owner) {
            return Err(invalid());
        }
        if self.store.is_some() {
            match (owner, &activation) {
                (OwnerKey::Manual { .. }, None) => {}
                (OwnerKey::Session { pid, token, .. }, Some(f))
                    if *pid == f.pid && *token == f.token && !f.image.is_empty() => {}
                _ => return Err(invalid()),
            }
        }
        let descriptor = module.descriptor();
        if !target.valid()
            || descriptor.access != ModuleAccess::Managed
            || config.feature != descriptor.id
            || config.version != descriptor.config_version
            || !module.editor().accepts_config(&config)
            || !descriptor.target_scopes.contains(&target.scope())
        {
            return Err(invalid());
        }
        let changes = module.fields(&target, &config)?;
        if changes.len() > 64 {
            return Err(invalid());
        }
        if changes.is_empty() {
            return self.release(owner);
        }
        let mut keys = BTreeSet::new();
        let mut prepared = Vec::new();
        for change in changes {
            if change.key.target != target
                || !change.key.target.valid()
                || !keys.insert(change.key.clone())
                || change.desired.len() > 65536
            {
                return Err(invalid());
            }
            if let Some(field) = self.fields.get(&change.key) {
                if field.attempted.is_some() {
                    return Err(ModuleError::new(ErrorKind::Verification, "uncertain write"));
                }
                if field.owners.iter().any(|other| other != owner)
                    && field.written != change.desired
                {
                    return Err(ModuleError::new(ErrorKind::Conflict, "acquire"));
                }
                if field.module.read(&change.key)? != field.written {
                    return Err(ModuleError::new(ErrorKind::ExternalChange, "acquire"));
                }
                prepared.push((change, field.written.clone()));
            } else {
                let baseline = module.read(&change.key)?;
                if baseline.len() > 65536 {
                    return Err(invalid());
                }
                prepared.push((change, baseline));
            }
        }
        let new_fields = prepared
            .iter()
            .filter(|(change, _)| !self.fields.contains_key(&change.key))
            .count();
        let mut bytes: usize = self
            .fields
            .values()
            .map(|field| field.baseline.len() + field.written.len())
            .sum();
        for (change, current) in &prepared {
            bytes = if let Some(field) = self.fields.get(&change.key) {
                bytes - field.written.len() + change.desired.len()
            } else {
                bytes + current.len() + change.desired.len()
            };
        }
        if self.fields.len() + new_fields > 256 || bytes > 4 * 1024 * 1024 {
            return Err(invalid());
        }
        self.records.insert(
            owner.clone(),
            OwnerRecord {
                owner: owner.clone(),
                config,
                activation,
            },
        );
        for (change, current) in &prepared {
            let field = self
                .fields
                .entry(change.key.clone())
                .or_insert_with(|| OwnedField {
                    module: module.clone(),
                    baseline: current.clone(),
                    written: current.clone(),
                    desired: change.desired.clone(),
                    attempted: None,
                    owners: BTreeSet::new(),
                });
            field.desired = change.desired.clone();
            field.owners.insert(owner.clone());
        }
        self.persist()?;
        for (FieldChange { key, desired }, current) in prepared {
            if self.fields[&key].written == desired {
                continue;
            }
            self.fields.get_mut(&key).unwrap().attempted = Some(desired.clone());
            self.persist()?;
            let write = module.write(&key, &current, &desired);
            let observed = module.read(&key);
            let field = self.fields.get_mut(&key).unwrap();
            match observed {
                Ok(actual) if actual == desired => {
                    field.written = desired;
                    field.attempted = None;
                }
                Ok(actual) if actual == current => {
                    field.attempted = None;
                    self.persist()?;
                    return Err(write
                        .err()
                        .unwrap_or_else(|| ModuleError::new(ErrorKind::Verification, "write")));
                }
                _ => {
                    if self.store.is_some() {
                        self.pending.insert(owner.clone());
                    }
                    return Err(write
                        .err()
                        .unwrap_or_else(|| ModuleError::new(ErrorKind::Verification, "write")));
                }
            }
            self.persist()?;
            write?;
        }
        let removed: Vec<_> = self
            .fields
            .iter()
            .filter(|(key, f)| f.owners.contains(owner) && !keys.contains(*key))
            .map(|(key, _)| key.clone())
            .collect();
        for key in removed {
            self.release_field(owner, &key)?;
        }
        Ok(())
    }
    pub(crate) fn recover_owner(
        &mut self,
        owner: &OwnerKey,
        intent: RecoveryIntent,
    ) -> Result<(), ModuleError> {
        if intent == RecoveryIntent::Unknown {
            self.pending.insert(owner.clone());
            return Ok(());
        }
        let keys: Vec<_> = self
            .fields
            .iter()
            .filter(|(_, f)| f.owners.contains(owner))
            .map(|(key, _)| key.clone())
            .collect();
        let result = (|| {
            for key in &keys {
                let field = self.fields.get(key).unwrap();
                let actual = field.module.read(key)?;
                if actual != field.written
                    && field.attempted.as_ref() != Some(&actual)
                    && !(intent == RecoveryIntent::Restore && actual == field.baseline)
                {
                    return Err(ModuleError::new(ErrorKind::ExternalChange, "recover field"));
                }
                let field = self.fields.get_mut(key).unwrap();
                field.written = actual;
                field.attempted = None;
            }
            if !keys.is_empty() {
                self.persist()?;
            }
            if intent == RecoveryIntent::Keep {
                for key in &keys {
                    let field = &self.fields[key];
                    if field.written == field.desired {
                        continue;
                    }
                    let (module, current, desired) = (
                        field.module.clone(),
                        field.written.clone(),
                        field.desired.clone(),
                    );
                    self.fields.get_mut(key).unwrap().attempted = Some(desired.clone());
                    self.persist()?;
                    let result = module.write(key, &current, &desired);
                    let actual = module.read(key)?;
                    if actual != desired {
                        return Err(result.err().unwrap_or_else(|| {
                            ModuleError::new(ErrorKind::Verification, "recover write")
                        }));
                    }
                    let field = self.fields.get_mut(key).unwrap();
                    field.written = actual;
                    field.attempted = None;
                    self.persist()?;
                    result?;
                }
            } else {
                self.release(owner)?;
            }
            self.pending.remove(owner);
            Ok(())
        })();
        if result.is_err() {
            self.pending.insert(owner.clone());
        }
        self.remember(owner, &result);
        result
    }
    fn release_field(&mut self, owner: &OwnerKey, key: &ResourceKey) -> Result<(), ModuleError> {
        let field = self.fields.get(key).expect("owned field");
        if field.attempted.is_some() || field.module.read(key)? != field.written {
            return Err(ModuleError::new(ErrorKind::ExternalChange, "release"));
        }
        if field.owners.len() > 1 {
            self.fields.get_mut(key).unwrap().owners.remove(owner);
            let record = self.records.get(owner).cloned();
            if !self.owners().contains(owner) {
                self.records.remove(owner);
            }
            if let Err(error) = self.persist() {
                self.fields
                    .get_mut(key)
                    .unwrap()
                    .owners
                    .insert(owner.clone());
                if let Some(record) = record {
                    self.records.insert(owner.clone(), record);
                }
                return Err(error);
            }
            return Ok(());
        }
        if field.written != field.baseline {
            let (module, written, baseline) = (
                field.module.clone(),
                field.written.clone(),
                field.baseline.clone(),
            );
            self.fields.get_mut(key).unwrap().attempted = Some(baseline.clone());
            self.persist()?;
            let result = module.write(key, &written, &baseline);
            let field = self.fields.get_mut(key).unwrap();
            match module.read(key) {
                Ok(actual) if actual == baseline => {
                    field.written = actual;
                    field.attempted = None;
                }
                Ok(actual) if actual == written => {
                    field.attempted = None;
                    self.persist()?;
                    return Err(result
                        .err()
                        .unwrap_or_else(|| ModuleError::new(ErrorKind::Verification, "restore")));
                }
                _ => {
                    return Err(result
                        .err()
                        .unwrap_or_else(|| ModuleError::new(ErrorKind::Verification, "restore")))
                }
            }
            self.persist()?;
            result?;
        }
        let field = self.fields.remove(key).unwrap();
        let record = self.records.get(owner).cloned();
        if !self.owners().contains(owner) {
            self.records.remove(owner);
        }
        if let Err(error) = self.persist() {
            self.fields.insert(key.clone(), field);
            if let Some(record) = record {
                self.records.insert(owner.clone(), record);
            }
            return Err(error);
        }
        self.restoration_effect = Some(field.module.descriptor().effect);
        Ok(())
    }
    pub fn release(&mut self, owner: &OwnerKey) -> Result<(), ModuleError> {
        let keys: Vec<_> = self
            .fields
            .iter()
            .filter(|(_, f)| f.owners.contains(owner))
            .map(|(key, _)| key.clone())
            .collect();
        let mut failed = None;
        for key in keys {
            if let Err(error) = self.release_field(owner, &key) {
                failed = Some(error);
            }
        }
        let result = if let Some(error) = failed {
            if self.store.is_some() {
                self.pending.insert(owner.clone());
            }
            Err(error)
        } else {
            self.pending.remove(owner);
            Ok(())
        };
        self.remember(owner, &result);
        result
    }
    pub fn release_if_exited(
        &mut self,
        owner: &OwnerKey,
        evidence: LifetimeEvidence,
    ) -> Result<bool, ModuleError> {
        if evidence != LifetimeEvidence::Exited {
            return Ok(false);
        }
        if let Some(facts) = self
            .records
            .get(owner)
            .and_then(|record| record.activation.as_ref())
            .cloned()
        {
            let expired:Vec<_>=self.fields.iter().filter(|(key,field)|field.owners.contains(owner) && matches!(&key.target,TargetIdentity::Process{pid,creation_token,image_path} if *pid==facts.pid && *creation_token==facts.token && image_path==&facts.image)).map(|(key,_)|key.clone()).collect();
            for key in expired {
                let field = self.fields.remove(&key).unwrap();
                let records = self.records.clone();
                let pending = self.pending.clone();
                let owners = self.owners();
                self.records.retain(|owner, _| owners.contains(owner));
                self.pending.retain(|owner| owners.contains(owner));
                if let Err(error) = self.persist() {
                    self.fields.insert(key, field);
                    self.records = records;
                    self.pending = pending;
                    self.errors.insert(owner.clone(), error.clone());
                    return Err(error);
                }
            }
        }
        self.recover_owner(owner, RecoveryIntent::Restore)?;
        Ok(true)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownResult {
    Complete,
    RecoveryRequired,
    TimedOut,
    Faulted,
}
