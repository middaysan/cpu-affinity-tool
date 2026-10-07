//! Bounded, typed recovery persistence. Envelopes are codec boundaries only.
use super::{
    codec::{EnvelopeKind, ModuleEnvelope},
    contract::*,
    coordinator::CoordinatorSnapshot,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Write, sync::Arc};

pub const MAX_JOURNAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESOURCES: usize = 4096;
const MAX_OWNERS: usize = 256;
const MAX_SOURCES: usize = 8448;
const FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JournalStamp {
    Operation {
        operation: OperationId,
        config_generation: u64,
    },
    Recovery,
    Shutdown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JournalPhase {
    Transition,
    BeforeWrite { step: u32, effect: Effect },
    VerifiedWrite { step: u32, effect: Effect },
    Recovery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceRef(pub u32);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalSource {
    pub snapshot: ModuleEnvelope,
    pub config: Option<ModuleEnvelope>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalOwner {
    pub owner: OwnerId,
    pub source: SourceRef,
    pub mode: ActivationMode,
    pub resources: Vec<ResourceKey>,
    pub status: ModuleStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalResource {
    pub key: ResourceKey,
    pub original: SourceRef,
    pub current: SourceRef,
    pub baseline: DesiredStateToken,
    pub desired: DesiredStateToken,
    pub confirmed: DesiredStateToken,
    pub attempted: Vec<DesiredStateToken>,
    pub owners: Vec<OwnerId>,
    pub status: ModuleStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalDocument {
    pub format: u32,
    pub machine: MachineIdentity,
    pub revision: u64,
    pub stamp: JournalStamp,
    pub phase: JournalPhase,
    pub sources: Vec<JournalSource>,
    pub owners: Vec<JournalOwner>,
    pub resources: Vec<JournalResource>,
    pub restoration_effect: Option<Effect>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalIoKind {
    Busy,
    Denied,
    Unsafe,
    Unavailable,
    Capacity,
    Io,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalIoError {
    pub kind: JournalIoKind,
    pub operation: &'static str,
    pub native_code: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalError {
    TooLarge,
    InvalidData,
    UnsupportedFormat(u32),
    MachineMismatch,
    InvalidEvidence(&'static str),
    RevisionExhausted,
    Io(JournalIoError),
    Blocked,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalBlock {
    pub reason: JournalError,
    pub raw: Option<Arc<[u8]>>,
    pub proposed: Option<Arc<[u8]>>,
    pub observed_size: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalLoad {
    Clean,
    Known(JournalDocument),
    Blocked(JournalBlock),
}
pub enum StoredJournal {
    Absent,
    Bytes(Vec<u8>),
    Oversized { size: u64 },
}
/// Construct/drop the store and native lease on the serial worker. No Send bound.
pub(super) trait JournalStore {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError>;
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError>;
    fn validate_authority(&mut self) -> Result<(), JournalIoError>;
}
impl<T: JournalStore + ?Sized> JournalStore for Box<T> {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
        (**self).read()
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
        (**self).publish(bytes)
    }
    fn validate_authority(&mut self) -> Result<(), JournalIoError> {
        (**self).validate_authority()
    }
}
pub(super) struct DurableSession<S: JournalStore> {
    store: S,
    machine: MachineIdentity,
    load: JournalLoad,
    last: Option<JournalDocument>,
    raw: Option<Arc<[u8]>>,
    blocked: bool,
}

struct BudgetWriter {
    bytes: Vec<u8>,
    count: usize,
    retain: bool,
    exceeded: bool,
}
impl BudgetWriter {
    fn new(retain: bool) -> Self {
        Self {
            bytes: vec![],
            count: 0,
            retain,
            exceeded: false,
        }
    }
}
impl Write for BudgetWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let Some(next) = self
            .count
            .checked_add(bytes.len())
            .filter(|next| *next <= MAX_JOURNAL_BYTES)
        else {
            self.exceeded = true;
            return Err(std::io::Error::other("journal capacity"));
        };
        if self.retain {
            self.bytes
                .try_reserve_exact(bytes.len())
                .map_err(|_| std::io::Error::other("journal allocation"))?;
            self.bytes.extend_from_slice(bytes);
        }
        self.count = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_bounded<T: Serialize>(value: &T, retain: bool) -> Result<BudgetWriter, JournalError> {
    let mut writer = BudgetWriter::new(retain);
    serde_json::to_writer(&mut writer, value).map_err(|_| {
        if writer.exceeded {
            JournalError::TooLarge
        } else {
            JournalError::InvalidData
        }
    })?;
    Ok(writer)
}
fn charge<T: Serialize>(budget: &mut usize, value: &T) -> Result<(), JournalError> {
    *budget = budget
        .checked_add(encode_bounded(value, false)?.count)
        .filter(|next| *next <= MAX_JOURNAL_BYTES)
        .ok_or(JournalError::TooLarge)?;
    Ok(())
}
fn distinct<T: Ord>(values: impl IntoIterator<Item = T>) -> bool {
    let mut seen = BTreeSet::new();
    values.into_iter().all(|value| seen.insert(value))
}
fn same_semantics(left: &DesiredStateToken, right: &DesiredStateToken) -> bool {
    left.semantics == right.semantics && left.version == right.version
}
fn bound_machine(target: &TargetIdentity, machine: &MachineIdentity) -> bool {
    target.resource_target(TargetScope::Machine) == Some(TargetIdentity::Machine(machine.clone()))
}
fn add_source(
    sources: &mut Vec<JournalSource>,
    budget: &mut usize,
    snapshot: &ModuleEnvelope,
    config: Option<&ModuleEnvelope>,
) -> Result<SourceRef, JournalError> {
    if let Some(index) = sources
        .iter()
        .position(|source| source.snapshot == *snapshot && source.config.as_ref() == config)
    {
        return Ok(SourceRef(index as u32));
    }
    if sources.len() >= MAX_SOURCES {
        return Err(JournalError::TooLarge);
    }
    snapshot
        .to_bytes()
        .map_err(|_| JournalError::InvalidEvidence("snapshot envelope"))?;
    if let Some(config) = config {
        config
            .to_bytes()
            .map_err(|_| JournalError::InvalidEvidence("config envelope"))?;
    }
    let source = JournalSource {
        snapshot: snapshot.clone(),
        config: config.cloned(),
    };
    charge(budget, &source)?;
    let reference = SourceRef(sources.len() as u32);
    sources.push(source);
    Ok(reference)
}

impl JournalDocument {
    pub(super) fn capture(
        machine: MachineIdentity,
        revision: u64,
        stamp: JournalStamp,
        phase: JournalPhase,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<Self, JournalError> {
        if snapshot.resources.len() > MAX_RESOURCES || snapshot.owners.len() > MAX_OWNERS {
            return Err(JournalError::TooLarge);
        }
        let mut document = Self {
            format: FORMAT,
            machine,
            revision,
            stamp,
            phase,
            sources: vec![],
            owners: vec![],
            resources: vec![],
            restoration_effect: snapshot.restoration_effect,
        };
        let mut budget = 0;
        for resource in &snapshot.resources {
            let original = add_source(
                &mut document.sources,
                &mut budget,
                &resource.baseline_snapshot,
                resource.baseline_config.as_deref(),
            )?;
            let current = add_source(
                &mut document.sources,
                &mut budget,
                &resource.current_snapshot,
                resource.current_config.as_deref(),
            )?;
            let stored = JournalResource {
                key: resource.key.clone(),
                original,
                current,
                baseline: resource.baseline.clone(),
                desired: resource.desired.clone(),
                confirmed: resource.confirmed.clone(),
                attempted: resource.possible.clone(),
                owners: resource.owners.clone(),
                status: resource.status,
            };
            charge(&mut budget, &stored)?;
            document.resources.push(stored);
        }
        for owner in &snapshot.owners {
            let source = add_source(
                &mut document.sources,
                &mut budget,
                &owner.snapshot,
                owner.config.as_deref(),
            )?;
            let stored = JournalOwner {
                owner: owner.owner,
                source,
                mode: owner.mode,
                resources: owner.resources.clone(),
                status: owner.status,
            };
            charge(&mut budget, &stored)?;
            document.owners.push(stored);
        }
        document.validate()?;
        encode_bounded(&document, false)?;
        Ok(document)
    }
    pub(super) fn validate(&self) -> Result<(), JournalError> {
        let invalid = JournalError::InvalidEvidence;
        if self.format != FORMAT {
            return Err(JournalError::UnsupportedFormat(self.format));
        }
        if self.revision == 0 {
            return Err(invalid("zero revision"));
        }
        if matches!(
            self.stamp,
            JournalStamp::Operation {
                config_generation: 0,
                ..
            }
        ) {
            return Err(invalid("zero operation generation"));
        }
        if self.resources.len() > MAX_RESOURCES
            || self.owners.len() > MAX_OWNERS
            || self.sources.len() > MAX_SOURCES
        {
            return Err(JournalError::TooLarge);
        }
        if !distinct(self.resources.iter().map(|resource| &resource.key))
            || !distinct(self.owners.iter().map(|owner| owner.owner))
        {
            return Err(invalid("duplicate ledger identity"));
        }
        for source in &self.sources {
            let target = source
                .snapshot
                .target
                .as_ref()
                .ok_or(invalid("snapshot target"))?;
            source
                .snapshot
                .to_bytes()
                .map_err(|_| invalid("snapshot envelope"))?;
            if source.snapshot.kind != EnvelopeKind::Snapshot
                || !bound_machine(target, &self.machine)
            {
                return Err(invalid("snapshot machine binding"));
            }
            if let Some(config) = &source.config {
                config.to_bytes().map_err(|_| invalid("config envelope"))?;
                if config.feature != source.snapshot.feature || config.kind != EnvelopeKind::Config
                {
                    return Err(invalid("source config identity"));
                }
            }
        }
        for resource in &self.resources {
            if !bound_machine(&resource.key.target, &self.machine)
                || !distinct(resource.owners.iter())
                || resource.attempted.len() > 256
                || !distinct(resource.attempted.iter().map(|token| {
                    (
                        token.semantics.clone(),
                        token.version,
                        token.bytes().to_vec(),
                    )
                }))
            {
                return Err(invalid("resource binding or attempts"));
            }
            for reference in [resource.original, resource.current] {
                let source = self
                    .sources
                    .get(reference.0 as usize)
                    .ok_or(invalid("unknown source reference"))?;
                if source
                    .snapshot
                    .target
                    .as_ref()
                    .and_then(|target| target.resource_target(resource.key.target.scope()))
                    .as_ref()
                    != Some(&resource.key.target)
                {
                    return Err(invalid("source resource target"));
                }
            }
            if !same_semantics(&resource.baseline, &resource.desired)
                || !same_semantics(&resource.confirmed, &resource.desired)
                || resource
                    .attempted
                    .iter()
                    .any(|token| !same_semantics(token, &resource.desired))
            {
                return Err(invalid("canonical token semantics"));
            }
            for owner in &resource.owners {
                if !self.owners.iter().any(|record| {
                    record.owner == *owner && record.resources.contains(&resource.key)
                }) {
                    return Err(invalid("resource owner reference"));
                }
            }
        }
        for owner in &self.owners {
            let source = self
                .sources
                .get(owner.source.0 as usize)
                .ok_or(invalid("unknown owner source"))?;
            if source.config.is_none()
                || owner.resources.is_empty()
                || !distinct(owner.resources.iter())
            {
                return Err(invalid("owner intent"));
            }
            for key in &owner.resources {
                let resource = self
                    .resources
                    .iter()
                    .find(|record| record.key == *key)
                    .ok_or(invalid("owner resource reference"))?;
                if !resource.owners.contains(&owner.owner)
                    && !matches!(
                        owner.status,
                        ModuleStatus::Restoring
                            | ModuleStatus::RecoveryRequired
                            | ModuleStatus::PartialFailure
                            | ModuleStatus::PermissionDenied
                            | ModuleStatus::ExternalChange
                    )
                {
                    return Err(invalid("unclaimed active owner"));
                }
            }
        }
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, JournalError> {
        self.validate()?;
        Ok(encode_bounded(self, true)?.bytes)
    }
    pub fn load(bytes: Vec<u8>, machine: &MachineIdentity) -> JournalLoad {
        let size = bytes.len() as u64;
        if bytes.len() > MAX_JOURNAL_BYTES {
            return JournalLoad::Blocked(JournalBlock {
                reason: JournalError::TooLarge,
                raw: None,
                proposed: None,
                observed_size: Some(size),
            });
        }
        let raw: Arc<[u8]> = Arc::from(bytes);
        #[derive(Deserialize)]
        struct Header {
            format: u32,
        }
        let decoded = (|| {
            let header: Header =
                serde_json::from_slice(&raw).map_err(|_| JournalError::InvalidData)?;
            if header.format != FORMAT {
                return Err(JournalError::UnsupportedFormat(header.format));
            }
            let document: Self =
                serde_json::from_slice(&raw).map_err(|_| JournalError::InvalidData)?;
            document.validate()?;
            if document.machine != *machine {
                return Err(JournalError::MachineMismatch);
            }
            Ok(document)
        })();
        match decoded {
            Ok(document) => JournalLoad::Known(document),
            Err(reason) => JournalLoad::Blocked(JournalBlock {
                reason,
                raw: Some(raw),
                proposed: None,
                observed_size: Some(size),
            }),
        }
    }
}
impl<S: JournalStore> DurableSession<S> {
    pub(super) fn bootstrap(mut store: S, machine: MachineIdentity) -> Self {
        let mut original_raw = None;
        let load = match store.validate_authority().and_then(|_| store.read()) {
            Ok(StoredJournal::Absent) => JournalLoad::Clean,
            Ok(StoredJournal::Bytes(bytes)) => {
                if bytes.len() <= MAX_JOURNAL_BYTES {
                    original_raw = Some(Arc::from(bytes.clone()));
                }
                JournalDocument::load(bytes, &machine)
            }
            Ok(StoredJournal::Oversized { size }) => JournalLoad::Blocked(JournalBlock {
                reason: JournalError::TooLarge,
                raw: None,
                proposed: None,
                observed_size: Some(size),
            }),
            Err(error) => JournalLoad::Blocked(JournalBlock {
                reason: JournalError::Io(error),
                raw: None,
                proposed: None,
                observed_size: None,
            }),
        };
        let last = match &load {
            JournalLoad::Known(document) => Some(document.clone()),
            _ => None,
        };
        let raw = original_raw;
        let blocked = matches!(load, JournalLoad::Blocked(_));
        Self {
            store,
            machine,
            load,
            last,
            raw,
            blocked,
        }
    }
    pub(super) fn load(&self) -> &JournalLoad {
        &self.load
    }
    pub(super) fn last_document(&self) -> Option<&JournalDocument> {
        self.last.as_ref()
    }
    pub(super) fn block(&mut self, reason: JournalError) {
        self.blocked = true;
        self.load = JournalLoad::Blocked(JournalBlock {
            reason,
            raw: self.raw.clone(),
            proposed: None,
            observed_size: self.raw.as_ref().map(|raw| raw.len() as u64),
        });
    }
    pub(super) fn commit(
        &mut self,
        stamp: JournalStamp,
        phase: JournalPhase,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), JournalError> {
        if self.blocked {
            return Err(JournalError::Blocked);
        }
        let mut proposed = None;
        let result: Result<(), JournalError> = (|| {
            self.store.validate_authority().map_err(JournalError::Io)?;
            let revision = self
                .last
                .as_ref()
                .map_or(Some(1), |last| last.revision.checked_add(1))
                .ok_or(JournalError::RevisionExhausted)?;
            let mut document =
                JournalDocument::capture(self.machine.clone(), revision, stamp, phase, snapshot)?;
            // A later command, readback, or process exit does not prove a
            // required restart occurred. Clearing needs a future explicit,
            // authoritative restart/acknowledgement contract.
            if let Some(previous) = self.last.as_ref().and_then(|last| last.restoration_effect) {
                if document.restoration_effect.is_none_or(|current| {
                    super::coordinator::effect_rank(previous)
                        > super::coordinator::effect_rank(current)
                }) {
                    document.restoration_effect = Some(previous);
                }
            }
            let bytes = document.to_bytes()?;
            let raw: Arc<[u8]> = Arc::from(bytes);
            proposed = Some(raw.clone());
            self.store.publish(&raw).map_err(JournalError::Io)?;
            self.raw = Some(raw);
            self.last = Some(document.clone());
            self.load = JournalLoad::Known(document);
            Ok(())
        })();
        if let Err(reason) = &result {
            self.block(reason.clone());
            if let JournalLoad::Blocked(block) = &mut self.load {
                block.proposed = proposed;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn machine() -> MachineIdentity {
        MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        }
    }
    fn stamp() -> JournalStamp {
        JournalStamp::Operation {
            operation: OperationId::new(1).unwrap(),
            config_generation: 1,
        }
    }
    #[test]
    fn empty_ownership_transition_still_has_machine_revision_and_operation_identity() {
        let result = JournalDocument::capture(
            machine(),
            1,
            stamp(),
            JournalPhase::Transition,
            &CoordinatorSnapshot::default(),
        );
        assert!(
            result.is_ok(),
            "zero-native-write transitions need a durable typed record too"
        );
        let document = result.unwrap();
        assert_eq!(document.machine, machine());
        assert_eq!(document.revision, 1);
        assert_eq!(document.stamp, stamp());
    }
    fn snapshot() -> CoordinatorSnapshot {
        use super::super::{
            catalog::Catalog,
            coordinator::{OwnerView, ResourceView},
        };
        use crate::app::features::tuning::testing::scalar::{ScalarConfig, ScalarModule};
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let target = TargetIdentity::Machine(machine());
        let config = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
        let prepared = catalog
            .prepare(&config, target.clone(), ActivationMode::Manual)
            .unwrap();
        let baseline = prepared.baseline_states().unwrap()[0].value.clone();
        let claim = prepared.resources()[0].clone();
        let envelope = Arc::new(prepared.snapshot_envelope().unwrap());
        let config = Some(Arc::new(config));
        let owner = OwnerId::new(1).unwrap();
        CoordinatorSnapshot {
            resources: vec![ResourceView {
                key: claim.key.clone(),
                desired: claim.desired,
                baseline: baseline.clone(),
                owners: vec![owner],
                status: ModuleStatus::Applying,
                baseline_source: prepared.descriptor().id.clone(),
                baseline_snapshot: envelope.clone(),
                baseline_config: config.clone(),
                current_source: prepared.descriptor().id.clone(),
                current_snapshot: envelope.clone(),
                current_config: config.clone(),
                confirmed: baseline,
                possible: vec![],
            }],
            owners: vec![OwnerView {
                owner,
                resources: vec![claim.key],
                status: ModuleStatus::Applying,
                mode: ActivationMode::Manual,
                source: prepared.descriptor().id.clone(),
                target,
                config,
                snapshot: envelope,
            }],
            restoration_effect: None,
        }
    }
    #[test]
    fn source_table_deduplicates_owners_baselines_and_writers_and_codec_roundtrips() {
        let document =
            JournalDocument::capture(machine(), 1, stamp(), JournalPhase::Transition, &snapshot())
                .unwrap();
        assert_eq!(document.sources.len(), 1);
        assert_eq!(document.resources[0].original, document.owners[0].source);
        let bytes = document.to_bytes().unwrap();
        assert_eq!(
            JournalDocument::load(bytes, &machine()),
            JournalLoad::Known(document)
        );
    }
    #[test]
    fn corrupt_unknown_and_mismatched_journals_preserve_exact_raw_bytes() {
        for raw in [
            b"{unfinished".to_vec(),
            b"{\"format\":9,\"future\":\"do not normalize\"}".to_vec(),
        ] {
            let JournalLoad::Blocked(block) = JournalDocument::load(raw.clone(), &machine()) else {
                panic!("blocked input");
            };
            assert_eq!(block.raw.unwrap().as_ref(), raw);
            assert!(block.proposed.is_none());
        }
        let document =
            JournalDocument::capture(machine(), 1, stamp(), JournalPhase::Transition, &snapshot())
                .unwrap();
        let raw = document.to_bytes().unwrap();
        let wrong = MachineIdentity {
            id: MachineId::new([2; 16]).unwrap(),
        };
        let JournalLoad::Blocked(block) = JournalDocument::load(raw.clone(), &wrong) else {
            panic!("machine mismatch");
        };
        assert_eq!(block.reason, JournalError::MachineMismatch);
        assert_eq!(block.raw.unwrap().as_ref(), raw);
    }
    #[test]
    fn legacy_boot_continuity_array_in_journal_retains_exact_raw_evidence() {
        use super::super::{
            catalog::Catalog,
            coordinator::{OwnerView, ResourceView},
        };
        use crate::app::features::tuning::testing::user_setting::{UserConfig, UserModule};
        let module = UserModule::new(None, true);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let target = TargetIdentity::Session(SessionIdentity {
            user: UserIdentity {
                machine: machine(),
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(17).unwrap(),
            boot: BootId::new([2; 16]).unwrap(),
        });
        let config = module
            .config_envelope(&UserConfig {
                value: 3,
                enabled: false,
            })
            .unwrap();
        let prepared = catalog
            .prepare(&config, target.clone(), ActivationMode::Manual)
            .unwrap();
        let envelope = Arc::new(prepared.snapshot_envelope().unwrap());
        let config = Some(Arc::new(config));
        let owner = OwnerId::new(1).unwrap();
        let baselines = prepared.baseline_states().unwrap();
        let resources = prepared
            .resources()
            .iter()
            .map(|claim| {
                let baseline = baselines
                    .iter()
                    .find(|state| state.key == claim.key)
                    .unwrap();
                ResourceView {
                    key: claim.key.clone(),
                    desired: claim.desired.clone(),
                    baseline: baseline.value.clone(),
                    confirmed: baseline.value.clone(),
                    owners: vec![owner],
                    status: ModuleStatus::Applying,
                    baseline_source: prepared.descriptor().id.clone(),
                    baseline_snapshot: envelope.clone(),
                    baseline_config: config.clone(),
                    current_source: prepared.descriptor().id.clone(),
                    current_snapshot: envelope.clone(),
                    current_config: config.clone(),
                    possible: vec![],
                }
            })
            .collect::<Vec<_>>();
        let ownership = CoordinatorSnapshot {
            owners: vec![OwnerView {
                owner,
                resources: resources
                    .iter()
                    .map(|resource| resource.key.clone())
                    .collect(),
                status: ModuleStatus::Applying,
                mode: ActivationMode::Manual,
                source: prepared.descriptor().id.clone(),
                target,
                config,
                snapshot: envelope,
            }],
            resources,
            restoration_effect: None,
        };
        let document =
            JournalDocument::capture(machine(), 1, stamp(), JournalPhase::Transition, &ownership)
                .unwrap();
        assert_eq!(
            JournalDocument::load(document.to_bytes().unwrap(), &machine()),
            JournalLoad::Known(document.clone())
        );
        let mut wire = serde_json::to_value(document).unwrap();
        let legacy = [2; 16];
        wire["sources"][0]["snapshot"]["target"]["Session"]["boot"] = serde_json::json!(legacy);
        wire["resources"][0]["key"]["target"]["Session"]["boot"] = serde_json::json!(legacy);
        let raw = serde_json::to_vec(&wire).unwrap();
        let JournalLoad::Blocked(block) = JournalDocument::load(raw.clone(), &machine()) else {
            panic!("legacy boot bytes must not become native continuity authority");
        };
        assert_eq!(block.reason, JournalError::InvalidData);
        assert_eq!(block.raw.unwrap().as_ref(), raw);
        assert!(block.proposed.is_none());
    }

    #[test]
    fn cross_references_and_token_bounds_cannot_bypass_deserialization() {
        let mut document =
            JournalDocument::capture(machine(), 1, stamp(), JournalPhase::Transition, &snapshot())
                .unwrap();
        document.resources[0].original = SourceRef(99);
        assert!(document.to_bytes().is_err());
        let mut wire = serde_json::to_value(
            JournalDocument::capture(machine(), 1, stamp(), JournalPhase::Transition, &snapshot())
                .unwrap(),
        )
        .unwrap();
        wire["resources"][0]["desired"]["bytes"] = serde_json::json!([]);
        let raw = serde_json::to_vec(&wire).unwrap();
        let JournalLoad::Blocked(block) = JournalDocument::load(raw.clone(), &machine()) else {
            panic!("invalid token");
        };
        assert_eq!(block.raw.unwrap().as_ref(), raw);
    }
    #[test]
    fn aggregate_encoding_stops_at_limit_without_unbounded_output_growth() {
        let mut writer = BudgetWriter::new(true);
        assert_eq!(
            writer.write(&vec![0; MAX_JOURNAL_BYTES]).unwrap(),
            MAX_JOURNAL_BYTES
        );
        assert!(writer.write(&[1]).is_err());
        assert_eq!(writer.bytes.len(), MAX_JOURNAL_BYTES);
        let mut document =
            JournalDocument::capture(machine(), 1, stamp(), JournalPhase::Transition, &snapshot())
                .unwrap();
        // Each envelope fits its own codec, while the complete journal exceeds
        // the aggregate budget. Source payloads stay at the codec boundary.
        let original = document.sources[0].clone();
        document.sources.clear();
        for _ in 0..24 {
            let mut source = original.clone();
            source.snapshot.payload = serde_json::json!({"large":"x".repeat(200_000)});
            document.sources.push(source);
        }
        assert_eq!(document.to_bytes(), Err(JournalError::TooLarge));
    }
    #[derive(Default)]
    struct Memory {
        bytes: Option<Vec<u8>>,
        fail: bool,
        publishes: usize,
        unsafe_store: bool,
        oversized: Option<u64>,
    }
    #[derive(Clone, Default)]
    struct Store(Arc<std::sync::Mutex<Memory>>);
    impl JournalStore for Store {
        fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
            let memory = self.0.lock().unwrap();
            Ok(if let Some(size) = memory.oversized {
                StoredJournal::Oversized { size }
            } else {
                memory
                    .bytes
                    .clone()
                    .map_or(StoredJournal::Absent, StoredJournal::Bytes)
            })
        }
        fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
            let mut memory = self.0.lock().unwrap();
            if memory.fail {
                return Err(JournalIoError {
                    kind: JournalIoKind::Io,
                    operation: "sync publication",
                    native_code: Some(5),
                });
            }
            memory.publishes += 1;
            memory.bytes = Some(bytes.to_vec());
            Ok(())
        }
        fn validate_authority(&mut self) -> Result<(), JournalIoError> {
            if self.0.lock().unwrap().unsafe_store {
                Err(JournalIoError {
                    kind: JournalIoKind::Unsafe,
                    operation: "authority",
                    native_code: None,
                })
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn failed_publication_latches_and_preserves_previous_and_proposed_evidence() {
        let store = Store::default();
        let mut session =
            DurableSession::bootstrap(Box::new(store.clone()) as Box<dyn JournalStore>, machine());
        session
            .commit(stamp(), JournalPhase::Transition, &snapshot())
            .unwrap();
        let before = store.0.lock().unwrap().bytes.clone().unwrap();
        store.0.lock().unwrap().fail = true;
        assert!(session
            .commit(
                JournalStamp::Shutdown,
                JournalPhase::Transition,
                &CoordinatorSnapshot::default()
            )
            .is_err());
        let JournalLoad::Blocked(block) = session.load() else {
            panic!("publication must block");
        };
        assert_eq!(block.raw.as_ref().unwrap().as_ref(), before);
        assert!(block.proposed.is_some());
        assert_eq!(session.last_document().unwrap().revision, 1);
        store.0.lock().unwrap().fail = false;
        assert_eq!(
            session.commit(stamp(), JournalPhase::Transition, &snapshot()),
            Err(JournalError::Blocked)
        );
        assert_eq!(store.0.lock().unwrap().publishes, 1);
    }
    #[test]
    fn unsafe_oversized_unknown_store_never_gets_a_clean_fallback_or_publication() {
        for store in [
            Store(Arc::new(std::sync::Mutex::new(Memory {
                unsafe_store: true,
                ..Default::default()
            }))),
            Store(Arc::new(std::sync::Mutex::new(Memory {
                oversized: Some(MAX_JOURNAL_BYTES as u64 + 1),
                ..Default::default()
            }))),
            Store(Arc::new(std::sync::Mutex::new(Memory {
                bytes: Some(b"{\"format\":99}".to_vec()),
                ..Default::default()
            }))),
        ] {
            let mut session = DurableSession::bootstrap(store.clone(), machine());
            assert!(matches!(session.load(), JournalLoad::Blocked(_)));
            assert_eq!(
                session.commit(
                    stamp(),
                    JournalPhase::Transition,
                    &CoordinatorSnapshot::default()
                ),
                Err(JournalError::Blocked)
            );
            assert_eq!(store.0.lock().unwrap().publishes, 0);
        }
    }
    #[test]
    fn final_empty_transition_preserves_restart_effect_and_exact_loaded_bytes() {
        let store = Store::default();
        let snapshot = CoordinatorSnapshot {
            restoration_effect: Some(Effect::RestartRequired),
            ..Default::default()
        };
        let mut session = DurableSession::bootstrap(store.clone(), machine());
        session
            .commit(JournalStamp::Shutdown, JournalPhase::Transition, &snapshot)
            .unwrap();
        assert_eq!(
            session.last_document().unwrap().restoration_effect,
            Some(Effect::RestartRequired)
        );
        let mut raw = b" \n".to_vec();
        raw.extend(store.0.lock().unwrap().bytes.clone().unwrap());
        raw.extend(b" \n");
        store.0.lock().unwrap().bytes = Some(raw.clone());
        let mut session = DurableSession::bootstrap(store, machine());
        session.block(JournalError::InvalidEvidence("unknown module version"));
        let JournalLoad::Blocked(block) = session.load() else {
            panic!("blocked recovery");
        };
        assert_eq!(block.raw.as_ref().unwrap().as_ref(), raw);
    }
    #[test]
    fn a_later_transition_cannot_clear_or_weaken_pending_restart_effect() {
        let mut session = DurableSession::bootstrap(Store::default(), machine());
        session
            .commit(
                JournalStamp::Recovery,
                JournalPhase::Transition,
                &CoordinatorSnapshot {
                    restoration_effect: Some(Effect::RestartRequired),
                    ..Default::default()
                },
            )
            .unwrap();
        for effect in [
            None,
            Some(Effect::Immediate),
            Some(Effect::GameRestartRequired),
        ] {
            session
                .commit(
                    stamp(),
                    JournalPhase::Transition,
                    &CoordinatorSnapshot {
                        restoration_effect: effect,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(
                session.last_document().unwrap().restoration_effect,
                Some(Effect::RestartRequired),
                "a setting observation or unrelated operation does not prove restart occurred"
            );
        }
    }
}
