//! Worker-local durable authority. Only its factory crosses the thread boundary.
use super::{
    catalog::Catalog,
    contract::MachineIdentity,
    coordinator::Coordinator,
    coordinator::{CoordinatorSnapshot, GateError},
    journal::{DurableSession, JournalLoad, JournalPhase, JournalStamp, JournalStore},
};

pub(super) fn bootstrap(
    catalog: Catalog,
    machine: MachineIdentity,
    store: Box<dyn JournalStore>,
) -> (Coordinator, WorkerAuthority) {
    use super::{journal::JournalError, recovery::RecoveryPlan};
    let mut session = DurableSession::bootstrap(store, machine);
    let coordinator = match session.load().clone() {
        JournalLoad::Known(document) => match Coordinator::recover(catalog.clone(), &document) {
            Ok(coordinator) => {
                let plan = RecoveryPlan::resolve(&document, &mut UnavailableLifetime);
                if plan.blocked() {
                    session.block(JournalError::InvalidEvidence(
                        "recovery lifetime is unverified",
                    ));
                }
                coordinator
            }
            Err(_) => {
                // Keep the same catalog for read-only diagnostics. An unknown
                // codec may not turn retained recovery evidence into Clean.
                session.block(JournalError::InvalidEvidence(
                    "journal source hydration failed",
                ));
                Coordinator::new(catalog)
            }
        },
        JournalLoad::Clean | JournalLoad::Blocked(_) => Coordinator::new(catalog),
    };
    (coordinator, WorkerAuthority::Durable(Box::new(session)))
}

struct UnavailableLifetime;
impl super::recovery::RecoveryResolver for UnavailableLifetime {
    fn owner(
        &mut self,
        _: &super::journal::JournalOwner,
        _: &super::journal::JournalSource,
    ) -> super::recovery::OwnerAttachment {
        super::recovery::OwnerAttachment::Unknown
    }
    fn resource(&mut self, _: &super::contract::ResourceKey) -> super::recovery::ResourceLifetime {
        super::recovery::ResourceLifetime::Unknown
    }
}

pub(super) enum WorkerAuthority {
    ReadOnly,
    #[cfg(test)]
    Fake,
    Durable(Box<DurableSession<Box<dyn JournalStore>>>),
    Blocked(super::journal::JournalError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriterStatus {
    ReadOnly,
    #[cfg(test)]
    Simulator,
    Clean,
    Known {
        revision: u64,
        phase: JournalPhase,
    },
    Blocked(super::journal::JournalError),
}

impl WorkerAuthority {
    fn close_unverified_native(&mut self) {
        if let Self::Durable(session) = self {
            if !matches!(session.load(), JournalLoad::Blocked(_)) {
                session.block(super::journal::JournalError::InvalidEvidence(
                    "native lifetime and scope acceptance pending",
                ));
            }
        }
    }
    pub(super) fn status(&self) -> WriterStatus {
        match self {
            Self::ReadOnly => WriterStatus::ReadOnly,
            #[cfg(test)]
            Self::Fake => WriterStatus::Simulator,
            Self::Durable(session) => match session.load() {
                JournalLoad::Clean => WriterStatus::Clean,
                JournalLoad::Known(document) => WriterStatus::Known {
                    revision: document.revision,
                    phase: document.phase.clone(),
                },
                JournalLoad::Blocked(block) => WriterStatus::Blocked(block.reason.clone()),
            },
            Self::Blocked(reason) => WriterStatus::Blocked(reason.clone()),
        }
    }

    pub(super) fn commit(
        &mut self,
        stamp: JournalStamp,
        phase: JournalPhase,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        match self {
            Self::ReadOnly => Err(GateError::MutationDisabled),
            #[cfg(test)]
            Self::Fake => Ok(()),
            Self::Durable(session) => session
                .commit(stamp, phase, snapshot)
                .map_err(GateError::Durability),
            Self::Blocked(reason) => Err(GateError::Durability(reason.clone())),
        }
    }
}

#[cfg(windows)]
struct NativeJournalStore(os_api::tuning::TuningStore);

#[cfg(windows)]
impl NativeJournalStore {
    fn open_canonical() -> Result<Self, super::journal::JournalError> {
        use os_api::tuning::{TuningStore, TuningStoreAuthority};
        let store = TuningStore::open_canonical(super::journal::MAX_JOURNAL_BYTES)
            .map_err(|error| super::journal::JournalError::Io(native_error(error)))?;
        if store.authority() != TuningStoreAuthority::Canonical {
            return Err(super::journal::JournalError::InvalidEvidence(
                "canonical writer authority required",
            ));
        }
        Ok(Self(store))
    }
}

#[cfg(windows)]
impl JournalStore for NativeJournalStore {
    fn read(&mut self) -> Result<super::journal::StoredJournal, super::journal::JournalIoError> {
        use super::journal::StoredJournal;
        match self.0.read() {
            Ok(None) => Ok(StoredJournal::Absent),
            Ok(Some(bytes)) => Ok(StoredJournal::Bytes(bytes)),
            Err(os_api::tuning::TuningNativeError::Capacity { size, .. }) => {
                Ok(StoredJournal::Oversized { size })
            }
            Err(error) => Err(native_error(error)),
        }
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), super::journal::JournalIoError> {
        self.0.publish(bytes).map_err(native_error)
    }
    fn validate_authority(&mut self) -> Result<(), super::journal::JournalIoError> {
        self.0.validate_authority().map_err(native_error)
    }
}

/// This dormant factory has no caller-selected path or permissive authority
/// injection. F4 supplies actual lifetime/attachment producers before admission.
#[cfg(windows)]
pub(super) fn open_native(catalog: Catalog) -> (Coordinator, WorkerAuthority) {
    let opened = (|| {
        let identity = os_api::OS::tuning_machine_identity()
            .map_err(|error| super::journal::JournalError::Io(native_error(error)))?;
        let machine = native_machine(identity.source(), identity.bytes())?;
        let store = NativeJournalStore::open_canonical()?;
        Ok((machine, store))
    })();
    match opened {
        Ok((machine, store)) => {
            let (coordinator, mut authority) = bootstrap(catalog, machine, Box::new(store));
            authority.close_unverified_native();
            (coordinator, authority)
        }
        Err(reason) => (Coordinator::new(catalog), WorkerAuthority::Blocked(reason)),
    }
}

#[cfg(windows)]
fn native_error(error: os_api::tuning::TuningNativeError) -> super::journal::JournalIoError {
    use super::journal::{JournalIoError, JournalIoKind};
    use os_api::tuning::TuningNativeError as Native;
    let (kind, operation, native_code) = match error {
        Native::Busy => (JournalIoKind::Busy, "writer lease", None),
        Native::Unsupported(operation) => (JournalIoKind::Unavailable, operation, None),
        Native::UnsafeStore(operation) => (JournalIoKind::Unsafe, operation, None),
        Native::Poisoned => (JournalIoKind::Unsafe, "uncertain publication", None),
        Native::Capacity { .. } => (JournalIoKind::Capacity, "journal capacity", None),
        Native::Denied { operation, code } => (JournalIoKind::Denied, operation, Some(code)),
        Native::Io { operation, code } => (JournalIoKind::Io, operation, Some(code)),
    };
    JournalIoError {
        kind,
        operation,
        native_code,
    }
}

#[cfg(windows)]
fn native_machine(
    source: os_api::tuning::TuningMachineSource,
    bytes: &[u8],
) -> Result<super::contract::MachineIdentity, super::journal::JournalError> {
    use super::contract::{MachineId, MachineIdSource, MachineIdentity};
    if bytes.is_empty()
        || bytes.len() > os_api::tuning::MAX_TUNING_MACHINE_BYTES
        || bytes.iter().all(|byte| *byte == 0)
    {
        return Err(super::journal::JournalError::InvalidEvidence(
            "invalid native machine binding",
        ));
    }
    let source = match source {
        os_api::tuning::TuningMachineSource::Tpm => MachineIdSource::Tpm,
        os_api::tuning::TuningMachineSource::Uefi => MachineIdSource::Uefi,
    };
    MachineId::observed(source, bytes.to_vec())
        .map(|id| MachineIdentity { id })
        .map_err(|_| {
            super::journal::JournalError::InvalidEvidence("invalid native machine binding")
        })
}

#[cfg(all(test, windows))]
mod native_mapping_tests {
    use super::super::{
        contract::MachineIdSource,
        journal::{JournalError, JournalIoKind},
    };
    use super::*;
    use os_api::tuning::{TuningMachineSource, TuningNativeError};

    #[test]
    fn native_mapping_preserves_denial_busy_capacity_and_unavailable_reasons() {
        let cases = [
            (TuningNativeError::Busy, JournalIoKind::Busy, None),
            (
                TuningNativeError::Unsupported("machine producer"),
                JournalIoKind::Unavailable,
                None,
            ),
            (
                TuningNativeError::UnsafeStore("unsafe ancestor"),
                JournalIoKind::Unsafe,
                None,
            ),
            (TuningNativeError::Poisoned, JournalIoKind::Unsafe, None),
            (
                TuningNativeError::Capacity { size: 9, limit: 8 },
                JournalIoKind::Capacity,
                None,
            ),
            (
                TuningNativeError::Denied {
                    operation: "OpenFile",
                    code: 5,
                },
                JournalIoKind::Denied,
                Some(5),
            ),
            (
                TuningNativeError::Io {
                    operation: "SyncFile",
                    code: 112,
                },
                JournalIoKind::Io,
                Some(112),
            ),
        ];
        for (error, kind, code) in cases {
            let mapped = native_error(error.clone());
            assert_eq!(mapped.kind, kind);
            assert_eq!(mapped.native_code, code);
            if let TuningNativeError::Denied { operation, .. }
            | TuningNativeError::Io { operation, .. } = error
            {
                assert_eq!(mapped.operation, operation);
            }
        }
    }
    #[test]
    fn native_mapping_binds_entire_machine_buffer_and_source_without_a_fixed_uuid() {
        let bytes: Vec<u8> = (1..=37).collect();
        let machine = native_machine(TuningMachineSource::Tpm, &bytes)
            .expect("full native machine buffer is supported");
        assert_eq!(machine.id.bytes(), bytes);
        assert_eq!(machine.id.source(), MachineIdSource::Tpm);
        let uefi = native_machine(TuningMachineSource::Uefi, &bytes).unwrap();
        assert_ne!(machine, uefi);
        for invalid in [vec![], vec![0; 37], vec![1; 4097]] {
            assert!(matches!(
                native_machine(TuningMachineSource::Tpm, &invalid),
                Err(JournalError::InvalidEvidence(_))
            ));
        }
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::super::{
        contract::*,
        coordinator::{Command, ExecutionControl, WriteIntent},
        journal::*,
    };
    use super::*;
    use crate::app::features::tuning::testing::scalar::{ScalarConfig, ScalarModule};
    use std::sync::{Arc, Mutex};
    struct TestControl;
    impl ExecutionControl for TestControl {
        fn cancellation_requested(&self) -> bool {
            false
        }
        fn before_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn after_verified_write(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
            Ok(())
        }
        fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
            Ok(())
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
    }
    #[derive(Default)]
    struct Memory {
        bytes: Option<Vec<u8>>,
        publishes: usize,
    }
    struct Store(Arc<Mutex<Memory>>);
    impl JournalStore for Store {
        fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .bytes
                .clone()
                .map_or(StoredJournal::Absent, StoredJournal::Bytes))
        }
        fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
            let mut memory = self.0.lock().unwrap();
            memory.publishes += 1;
            memory.bytes = Some(bytes.to_vec());
            Ok(())
        }
        fn validate_authority(&mut self) -> Result<(), JournalIoError> {
            Ok(())
        }
    }
    fn machine() -> MachineIdentity {
        MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        }
    }
    fn catalog(module: &ScalarModule) -> Catalog {
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        catalog
    }
    fn record(module: &ScalarModule) -> JournalDocument {
        let mut coordinator = Coordinator::new(catalog(module));
        coordinator.handle(
            Command::Acquire {
                owner: OwnerId::new(1).unwrap(),
                config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Manual,
            },
            &mut TestControl,
        );
        JournalDocument::capture(
            machine(),
            3,
            JournalStamp::Recovery,
            JournalPhase::Recovery,
            &coordinator.snapshot(),
        )
        .unwrap()
    }
    #[test]
    fn bootstrap_retains_unknown_raw_evidence_without_clean_fallback_or_write() {
        let raw = b" \n{\"format\":99,\"opaque\":\"unchanged\"}\n ".to_vec();
        let memory = Arc::new(Mutex::new(Memory {
            bytes: Some(raw.clone()),
            ..Default::default()
        }));
        let module = ScalarModule::new(7);
        let (_, authority) =
            bootstrap(catalog(&module), machine(), Box::new(Store(memory.clone())));
        assert!(matches!(
            authority.status(),
            WriterStatus::Blocked(JournalError::UnsupportedFormat(99))
        ));
        let WorkerAuthority::Durable(session) = authority else {
            panic!("blocked session retains store/lease and raw evidence");
        };
        let JournalLoad::Blocked(block) = session.load() else {
            panic!("unknown format must block");
        };
        assert_eq!(block.raw.as_ref().unwrap().as_ref(), raw);
        assert_eq!(memory.lock().unwrap().publishes, 0);
    }
    #[test]
    fn bootstrap_uncertain_lifetime_imports_original_ledger_and_blocks_mutation() {
        let module = ScalarModule::new(7);
        let document = record(&module);
        let raw = document.to_bytes().unwrap();
        let reads = module.reads();
        let memory = Arc::new(Mutex::new(Memory {
            bytes: Some(raw.clone()),
            ..Default::default()
        }));
        let (coordinator, authority) =
            bootstrap(catalog(&module), machine(), Box::new(Store(memory.clone())));
        assert_eq!(coordinator.snapshot().resources.len(), 1);
        assert_eq!(
            coordinator.snapshot().resources[0].baseline,
            document.resources[0].baseline
        );
        assert_eq!(
            module.reads(),
            reads,
            "hydration and Unknown lifetime do not observe/recapture native values"
        );
        assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
        assert_eq!(memory.lock().unwrap().bytes.as_ref().unwrap(), &raw);
        assert_eq!(memory.lock().unwrap().publishes, 0);
    }
    #[test]
    fn bootstrap_unknown_codec_preserves_catalog_for_readonly_observation() {
        let module = ScalarModule::new(7);
        let mut document = record(&module);
        for source in &mut document.sources {
            source.snapshot.version = SchemaVersion::new(99).unwrap();
        }
        let raw = document.to_bytes().unwrap();
        let memory = Arc::new(Mutex::new(Memory {
            bytes: Some(raw.clone()),
            ..Default::default()
        }));
        let (mut coordinator, authority) =
            bootstrap(catalog(&module), machine(), Box::new(Store(memory.clone())));
        assert!(matches!(authority.status(), WriterStatus::Blocked(_)));
        let result = coordinator.handle(
            Command::Observe {
                feature: FeatureId::new("test_scalar").unwrap(),
                target: TargetIdentity::Machine(machine()),
            },
            &mut TestControl,
        );
        assert!(matches!(
            result.outcome,
            super::super::coordinator::Outcome::Observed(_)
        ));
        assert_eq!(memory.lock().unwrap().bytes.as_ref().unwrap(), &raw);
        assert_eq!(memory.lock().unwrap().publishes, 0);
    }
    #[test]
    fn native_preflight_closes_a_clean_store_until_lifetime_acceptance() {
        let memory = Arc::new(Mutex::new(Memory::default()));
        let module = ScalarModule::new(7);
        let (_, mut authority) =
            bootstrap(catalog(&module), machine(), Box::new(Store(memory.clone())));
        authority.close_unverified_native();
        assert!(
            matches!(authority.status(), WriterStatus::Blocked(_)),
            "Clean is storage state, not native target/write admission"
        );
        assert!(authority
            .commit(
                JournalStamp::Recovery,
                JournalPhase::Transition,
                &CoordinatorSnapshot::default()
            )
            .is_err());
        assert_eq!(memory.lock().unwrap().publishes, 0);
        assert!(
            matches!(authority, WorkerAuthority::Durable(_)),
            "closed native preflight still retains its context/lease"
        );
    }
}
