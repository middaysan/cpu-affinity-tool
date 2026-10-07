//! Bounded manual lab. Pure/isolated tests never issue canonical authority.
use super::*;
use std::{cell::RefCell, rc::Rc};

const OWNER: u64 = 1;
fn binding() -> ActivationBinding {
    ActivationBinding::Manual(ConfiguredIntent {
        profile: ProfileId::new(1).unwrap(),
        generation: ConfigGeneration::new(1).unwrap(),
    })
}
struct Intent {
    machine: MachineIdentity,
    source: JournalSource,
    resource: ResourceClaim,
    baseline: DesiredStateToken,
}
fn intent(machine: MachineIdentity) -> Intent {
    // Only the expected typed codec shape of this fixed lab request. This is
    // never hydrated as recovery state and grants no native write authority.
    let module = ScalarModule::new(7);
    let config = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let mut catalog = Catalog::default();
    catalog.register(module).unwrap();
    let operation = catalog
        .prepare(
            &config,
            TargetIdentity::Machine(machine.clone()),
            ActivationMode::Manual,
        )
        .unwrap();
    Intent {
        machine,
        source: JournalSource {
            config: Some(config),
            snapshot: operation.snapshot_envelope().unwrap(),
        },
        resource: operation.resources()[0].clone(),
        baseline: operation.baseline_states().unwrap()[0].value.clone(),
    }
}
fn manual_guard(document: &JournalDocument, intent: &Intent) -> Result<(), ()> {
    document.validate().map_err(|_| ())?;
    if document.machine != intent.machine
        || document.owners.len() != 1
        || document.resources.len() != 1
        || document.sources.as_slice() != [intent.source.clone()]
        || document.restoration_effect.is_some()
        || document.phase != JournalPhase::Recovery
        || document.stamp != JournalStamp::Recovery
    {
        return Err(());
    }
    let owner = &document.owners[0];
    let resource = &document.resources[0];
    if owner.owner != OwnerId::new(OWNER).unwrap()
        || owner.binding != binding()
        || owner.mode != ActivationMode::Manual
        || owner.source != super::super::journal::SourceRef(0)
        || owner.resources != [intent.resource.key.clone()]
        || resource.key != intent.resource.key
        || resource.owners != [owner.owner]
        || resource.original != owner.source
        || resource.current != owner.source
        || resource.baseline != intent.baseline
        || resource.desired != intent.resource.desired
        || resource.confirmed != intent.resource.desired
        || !resource.attempted.is_empty()
    {
        return Err(());
    }
    Ok(())
}
#[derive(Default)]
struct Memory {
    bytes: Option<Vec<u8>>,
    phases: Vec<JournalPhase>,
}
#[derive(Clone)]
struct MemoryStore(Rc<RefCell<Memory>>);
impl JournalStore for MemoryStore {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
        Ok(self
            .0
            .borrow()
            .bytes
            .clone()
            .map_or(StoredJournal::Absent, StoredJournal::Bytes))
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
        let JournalLoad::Known(document) = JournalDocument::load(bytes.to_vec(), &machine()) else {
            panic!("known memory transcript");
        };
        let mut state = self.0.borrow_mut();
        state.phases.push(document.phase);
        state.bytes = Some(bytes.to_vec());
        Ok(())
    }
    fn validate_authority(&mut self) -> Result<(), JournalIoError> {
        Ok(())
    }
}
fn authority(store: impl JournalStore + 'static, machine: MachineIdentity) -> WorkerAuthority {
    WorkerAuthority::Durable(Box::new(DurableSession::bootstrap(
        Box::new(store) as Box<dyn JournalStore>,
        machine,
    )))
}
struct LiveResource;
impl RecoveryResolver for LiveResource {
    fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
        panic!("Manual auto-attachment cannot invoke an owner resolver");
    }
    fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
        ResourceLifetime::VerifiedLive
    }
}
fn acquire(module: &ScalarModule, control: &mut RecoveryControl<'_>) -> Coordinator {
    let mut catalog = Catalog::default();
    catalog.register(module.clone()).unwrap();
    let mut coordinator = Coordinator::new(catalog);
    let result = coordinator.handle(
        Command::Acquire {
            owner: OwnerId::new(OWNER).unwrap(),
            binding: binding(),
            config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
            target: TargetIdentity::Machine(machine()),
            mode: ActivationMode::Manual,
        },
        control,
    );
    assert!(result.error.is_none());
    assert_eq!(result.outcome, Outcome::Applied(Effect::Immediate));
    coordinator
}
#[test]
fn manual_lab_durable_transcript_has_seven_publications_and_reloads_baseline_seven() {
    let module = ScalarModule::new(7);
    let store = MemoryStore(Rc::new(RefCell::new(Memory::default())));
    let mut session = authority(store.clone(), machine());
    let coordinator = acquire(
        &module,
        &mut RecoveryControl {
            authority: &mut session,
        },
    );
    assert_eq!(module.value(), 12);
    drop(coordinator);
    drop(session);
    let raw = store.0.borrow().bytes.clone().unwrap();
    let JournalLoad::Known(document) = JournalDocument::load(raw, &machine()) else {
        panic!("actual serialized transcript");
    };
    assert_eq!(document.resources[0].baseline.bytes(), 7i32.to_le_bytes());
    assert_eq!(document.resources[0].confirmed.bytes(), 12i32.to_le_bytes());
    let mut catalog = Catalog::default();
    catalog.register(module.clone()).unwrap();
    let mut recovered = Coordinator::recover(catalog, &document).unwrap();
    let mut session = authority(store.clone(), machine());
    let writes = module.writes();
    let plan = RecoveryPlan::resolve(&document, &mut LiveResource);
    let result = recovered.apply_recovery(
        &plan,
        &mut RecoveryControl {
            authority: &mut session,
        },
    );
    assert!(result.error.is_none());
    assert_eq!(module.writes(), writes);
    assert_eq!(module.value(), 12);
    let result = recovered.handle(
        Command::Release {
            owner: OwnerId::new(OWNER).unwrap(),
        },
        &mut RecoveryControl {
            authority: &mut session,
        },
    );
    assert!(result.error.is_none());
    assert_eq!(module.value(), 7);
    assert!(authority_document(&session).owners.is_empty());
    let state = store.0.borrow();
    assert_eq!(state.phases.len(), 7);
    assert!(matches!(
        state.phases.as_slice(),
        [
            JournalPhase::BeforeWrite { step: 0, .. },
            JournalPhase::VerifiedWrite { step: 0, .. },
            JournalPhase::Recovery,
            JournalPhase::Recovery,
            JournalPhase::BeforeWrite { step: 0, .. },
            JournalPhase::VerifiedWrite { step: 0, .. },
            JournalPhase::Recovery
        ]
    ));
}
#[test]
fn manual_lab_exact_current_manual_intent_is_checked_before_auto_attachment() {
    let module = ScalarModule::new(7);
    let store = MemoryStore(Rc::new(RefCell::new(Memory::default())));
    let mut session = authority(store, machine());
    let coordinator = acquire(
        &module,
        &mut RecoveryControl {
            authority: &mut session,
        },
    );
    let document = authority_document(&session);
    let current = intent(machine());
    assert_eq!(manual_guard(&document, &current), Ok(()));
    for field in 0..5 {
        let mut changed = document.clone();
        match field {
            0 => changed.machine.id = MachineId::new([9; 16]).unwrap(),
            1 => changed.resources[0].key.field = ResourceId::new("different_resource").unwrap(),
            2 => {
                changed.owners[0].binding = ActivationBinding::Manual(ConfiguredIntent {
                    profile: ProfileId::new(2).unwrap(),
                    generation: ConfigGeneration::new(1).unwrap(),
                })
            }
            3 => {
                changed.sources[0].config.as_mut().unwrap().payload =
                    serde_json::json!({"value":13})
            }
            _ => {
                changed.sources[0].snapshot.payload =
                    serde_json::json!({"baseline":12,"desired":12,"known":[12]})
            }
        }
        assert!(manual_guard(&changed, &current).is_err());
    }
    drop(coordinator);
}

struct NativeJournal {
    store: Rc<RefCell<TuningStore>>,
    publications: Rc<RefCell<usize>>,
    machine: Option<os_api::tuning::TuningMachineIdentity>,
}
impl JournalStore for NativeJournal {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
        self.validate_authority()?;
        Ok(self
            .store
            .borrow_mut()
            .read()
            .map_err(native_error)?
            .map_or(StoredJournal::Absent, StoredJournal::Bytes))
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
        self.validate_authority()?;
        self.store
            .borrow_mut()
            .publish(bytes)
            .map_err(native_error)?;
        *self.publications.borrow_mut() += 1;
        Ok(())
    }
    fn validate_authority(&mut self) -> Result<(), JournalIoError> {
        self.store
            .borrow_mut()
            .validate_authority()
            .map_err(native_error)?;
        if let Some(expected) = &self.machine {
            if os_api::OS::tuning_machine_identity().map_err(native_error)? != *expected {
                return Err(native_error(TuningNativeError::UnsafeStore(
                    "manual lab machine changed",
                )));
            }
        }
        Ok(())
    }
}
struct BudgetControl<'a> {
    base: RecoveryControl<'a>,
    store: Rc<RefCell<TuningStore>>,
}
impl ExecutionControl for BudgetControl<'_> {
    fn cancellation_requested(&self) -> bool {
        false
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.store
            .borrow()
            .lab_require_manual_write_budget()
            .map_err(|_| GateError::Rejected("manual lab budget"))?;
        self.base.before_write(intent)
    }
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.base.after_verified_write(intent)
    }
    fn publish(&mut self, snapshot: CoordinatorSnapshot) {
        self.base.publish(snapshot);
    }
    fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
        self.base.commit_transition(snapshot)
    }
}
fn native_module(
    target: &os_api::tuning::TuningLabScalarTarget,
    rehydrated: bool,
    writes: Arc<AtomicUsize>,
) -> Result<FileScalarModule, ()> {
    Ok(FileScalarModule {
        scalar: ScalarModule::new(7),
        target: PhysicalTarget::Native(Arc::new(target.io().map_err(|_| ())?)),
        pause_after_write: None,
        rehydrated,
        writes: Some(writes),
    })
}
struct OwnResource<'a> {
    target: &'a mut os_api::tuning::TuningLabScalarTarget,
    key: &'a ResourceKey,
}
impl RecoveryResolver for OwnResource<'_> {
    fn owner(&mut self, _: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
        OwnerAttachment::Unknown
    }
    fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
        if key == self.key && self.target.read() == Ok(12) {
            ResourceLifetime::VerifiedLive
        } else {
            ResourceLifetime::Unknown
        }
    }
}
fn recover_checked(
    catalog: Catalog,
    document: &JournalDocument,
    current: &Intent,
    target: &mut os_api::tuning::TuningLabScalarTarget,
    control: &mut impl ExecutionControl,
) -> Result<Coordinator, ()> {
    // Manual is auto-attached by RecoveryPlan. Its resolver does not authorize
    // configured intent, so the exact independent current intent comes first.
    manual_guard(document, current)?;
    let mut recovered = Coordinator::recover(catalog, document).map_err(|_| ())?;
    let plan = RecoveryPlan::resolve(
        document,
        &mut OwnResource {
            target,
            key: &current.resource.key,
        },
    );
    let result = recovered.apply_recovery(&plan, control);
    if result.error.is_some() {
        return Err(());
    }
    Ok(recovered)
}
fn disk_document(
    store: &Rc<RefCell<TuningStore>>,
    machine: &MachineIdentity,
) -> Result<JournalDocument, ()> {
    let raw = store.borrow_mut().read().map_err(|_| ())?.ok_or(())?;
    match JournalDocument::load(raw, machine) {
        JournalLoad::Known(document) => Ok(document),
        _ => Err(()),
    }
}
fn round_trip(
    store: Rc<RefCell<TuningStore>>,
    machine: MachineIdentity,
    native_machine: Option<os_api::tuning::TuningMachineIdentity>,
    target: &mut os_api::tuning::TuningLabScalarTarget,
) -> Result<(), ()> {
    let current = intent(machine.clone());
    let count = Rc::new(RefCell::new(0));
    let journal = || NativeJournal {
        store: store.clone(),
        publications: count.clone(),
        machine: native_machine.clone(),
    };
    acquire_native(
        store.clone(),
        machine.clone(),
        native_machine.clone(),
        count.clone(),
        target,
    )?;
    // Bootstrap rereads this exact retained store's disk. No saved snapshot or prepare.
    let mut session = authority(journal(), machine.clone());
    let WorkerAuthority::Durable(durable) = &session else {
        return Err(());
    };
    let JournalLoad::Known(document) = durable.load().clone() else {
        return Err(());
    };
    let recovery_writes = Arc::new(AtomicUsize::new(0));
    let mut catalog = Catalog::default();
    catalog
        .register(native_module(target, true, recovery_writes.clone())?)
        .map_err(|_| ())?;
    let mut control = BudgetControl {
        base: RecoveryControl {
            authority: &mut session,
        },
        store: store.clone(),
    };
    let mut recovered = recover_checked(catalog, &document, &current, target, &mut control)?;
    if recovery_writes.load(Ordering::SeqCst) != 0 || target.read() != Ok(12) {
        return Err(());
    }
    manual_guard(&disk_document(&store, &machine)?, &current)?;
    let result = recovered.handle(
        Command::Release {
            owner: OwnerId::new(OWNER).unwrap(),
        },
        &mut control,
    );
    if result.error.is_some()
        || target.read() != Ok(7)
        || recovery_writes.load(Ordering::SeqCst) != 1
    {
        return Err(());
    }
    let released = disk_document(&store, &machine)?;
    if !released.owners.is_empty()
        || !released.resources.is_empty()
        || !released.sources.is_empty()
        || released.revision != 7
        || *count.borrow() != 7
    {
        return Err(());
    }
    Ok(())
}
#[test]
fn manual_lab_actual_disk_reload_reattaches_without_writes_and_releases_original_seven() {
    let root = TuningTemporaryDirectory::new().unwrap();
    drop(TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap());
    let mut native = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
    native.lab_arm_manual_recovery_rollback().unwrap();
    let store = Rc::new(RefCell::new(native));
    let mut target = os_api::tuning::TuningLabScalarTarget::new().unwrap();
    let result = round_trip(store.clone(), machine(), None, &mut target);
    let physical = target.read().unwrap();
    target.restore().unwrap();
    target.cleanup().unwrap();
    store.borrow_mut().lab_rollback().unwrap();
    drop(store);
    root.cleanup().unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(physical, 7);
}
#[test]
fn manual_lab_native_disk_mismatch_or_unknown_refuses_recovery_writes_and_preserves_evidence() {
    for field in 0..6 {
        let root = TuningTemporaryDirectory::new().unwrap();
        drop(TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap());
        let mut native = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
        native.lab_arm_manual_recovery_rollback().unwrap();
        let store = Rc::new(RefCell::new(native));
        let mut target = os_api::tuning::TuningLabScalarTarget::new().unwrap();
        let count = Rc::new(RefCell::new(0));
        acquire_native(store.clone(), machine(), None, count.clone(), &mut target).unwrap();
        let mut document = disk_document(&store, &machine()).unwrap();
        match field {
            0 => document.machine.id = MachineId::new([9; 16]).unwrap(),
            1 => document.resources[0].key.field = ResourceId::new("wrong_resource").unwrap(),
            2 => {
                document.owners[0].binding = ActivationBinding::Manual(ConfiguredIntent {
                    profile: ProfileId::new(2).unwrap(),
                    generation: ConfigGeneration::new(1).unwrap(),
                })
            }
            3 => {
                document.sources[0].config.as_mut().unwrap().payload =
                    serde_json::json!({"value":13})
            }
            4 => {
                document.sources[0].snapshot.payload =
                    serde_json::json!({"baseline":12,"desired":12,"known":[12]})
            }
            _ => (),
        }
        let raw = if field == 5 {
            b"unknown bounded journal".to_vec()
        } else {
            serde_json::to_vec(&document).unwrap()
        };
        store.borrow_mut().publish(&raw).unwrap();
        let mut session = authority(
            NativeJournal {
                store: store.clone(),
                publications: count.clone(),
                machine: None,
            },
            machine(),
        );
        let WorkerAuthority::Durable(durable) = &session else {
            panic!("durable fixture");
        };
        let loaded = durable.load().clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let mut catalog = Catalog::default();
        catalog
            .register(native_module(&target, true, writes.clone()).unwrap())
            .unwrap();
        let result = match loaded {
            JournalLoad::Known(document) => recover_checked(
                catalog,
                &document,
                &intent(machine()),
                &mut target,
                &mut BudgetControl {
                    base: RecoveryControl {
                        authority: &mut session,
                    },
                    store: store.clone(),
                },
            ),
            _ => {
                drop(catalog);
                Err(())
            }
        };
        assert!(result.is_err());
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        assert_eq!(target.read().unwrap(), 12);
        assert_eq!(store.borrow_mut().read().unwrap(), Some(raw));
        assert_eq!(*count.borrow(), 3, "no recovery publication");
        drop(result);
        drop(session);
        target.restore().unwrap();
        target.cleanup().unwrap();
        store.borrow_mut().lab_rollback().unwrap();
        drop(store);
        root.cleanup().unwrap();
    }
}
fn host_selection(marker: Option<&str>) -> bool {
    marker == Some("owned-target-restore-only")
}
fn host_guard(
    authority: TuningStoreAuthority,
    opened: bool,
    first: &Result<Option<Vec<u8>>, TuningNativeError>,
    machine: &MachineIdentity,
) -> bool {
    authority == TuningStoreAuthority::Canonical
        && opened
        && matches!(first, Ok(None))
        && matches!(
            machine.id.source(),
            MachineIdSource::Tpm | MachineIdSource::Uefi
        )
}
fn report_line(result: &'static str, terminal: bool, verified: bool, rollback: bool) -> String {
    // Fixed labels only. Leading LF survives libtest's unterminated test prefix.
    format!("\n{{\"schema\":1,\"result\":\"{result}\",\"worker_terminal\":{terminal},\"verified\":{verified},\"rollback_verified\":{rollback}}}\n")
}
fn report(result: &'static str, terminal: bool, verified: bool, rollback: bool) -> bool {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(report_line(result, terminal, verified, rollback).as_bytes())
        .and_then(|_| stdout.flush())
        .is_ok()
}
fn empty_artifacts(snapshot: &os_api::tuning::TuningLabSnapshot) -> bool {
    use os_api::tuning::TuningLabArtifactKind::{Journal, Previous, Stage};
    [Journal, Stage, Previous]
        .into_iter()
        .all(|kind| snapshot.artifact(kind).is_none())
}
fn host_worker() -> (&'static str, bool, bool) {
    let refused = ("refused", false, false);
    let Ok(mut store) = TuningStore::open_canonical_existing(MAX_JOURNAL_BYTES) else {
        return refused;
    };
    let first = store.read();
    let Ok(initial) = store.lab_snapshot() else {
        return refused;
    };
    let Ok(native_machine) = os_api::OS::tuning_machine_identity() else {
        return refused;
    };
    let Ok(machine) =
        super::super::writer::native_machine(native_machine.source(), native_machine.bytes())
    else {
        return refused;
    };
    if !host_guard(
        store.authority(),
        initial.opened_canonical(),
        &first,
        &machine,
    ) || !empty_artifacts(&initial)
    {
        return refused;
    }
    if store.lab_arm_manual_recovery_rollback().is_err() {
        return refused;
    }
    let Ok(mut target) = os_api::tuning::TuningLabScalarTarget::new() else {
        // No publication occurred. A failed target constructor cannot prove 7
        // or mint cleanup authority; its temporary evidence stays retained.
        return ("rollback_unresolved", false, false);
    };
    let store = Rc::new(RefCell::new(store));
    let proof = round_trip(store.clone(), machine, Some(native_machine), &mut target);
    // Every terminal protocol return reaches the same independently retained
    // target first. Its Drop retains unknown evidence and never performs repair.
    if target.restore().is_err() || target.read() != Ok(7) || target.cleanup().is_err() {
        return ("rollback_unresolved", false, false);
    }
    // Canonical receipts may be consumed only after confirmed physical baseline.
    if store.borrow_mut().lab_rollback().is_err() {
        return ("rollback_unresolved", false, false);
    }
    let after = store.borrow().lab_snapshot();
    let Ok(after) = after else {
        return ("rollback_unresolved", false, false);
    };
    if !empty_artifacts(&after)
        || after.root_id() != initial.root_id()
        || after.lease_id() != initial.lease_id()
    {
        return ("rollback_unresolved", false, false);
    }
    drop(store);
    let Ok(mut reopened) = TuningStore::open_canonical_existing(MAX_JOURNAL_BYTES) else {
        return ("rollback_unresolved", false, false);
    };
    let absent = reopened.read() == Ok(None);
    let snapshot = reopened.lab_snapshot();
    let exact = snapshot.is_ok_and(|snapshot| {
        snapshot.opened_canonical()
            && empty_artifacts(&snapshot)
            && snapshot.root_id() == initial.root_id()
            && snapshot.lease_id() == initial.lease_id()
    });
    drop(reopened);
    if !absent || !exact {
        return ("rollback_unresolved", false, false);
    }
    if proof.is_ok() {
        ("verified_rolled_back", true, true)
    } else {
        ("failed", false, true)
    }
}
pub(super) fn run_host() {
    if !host_selection(
        std::env::var("CPU_AFFINITY_MANUAL_RECOVERY_HOST_LAB")
            .ok()
            .as_deref(),
    ) {
        report("refused", true, false, false);
        panic!("explicit manual recovery host selection required");
    }
    let result = super::super::publication_lab_tests::wait_worker(
        host_worker,
        Duration::from_secs(10),
        || {
            let _ = report("pending", false, false, false);
        },
    );
    let passed = match result {
        Ok(((result, verified, rollback), false)) => {
            let emitted = report(result, true, verified, rollback);
            emitted && result == "verified_rolled_back" && verified && rollback
        }
        Ok(((_, _, rollback), true)) => {
            report("timed_out", true, false, rollback);
            false
        }
        Err(()) => {
            report("worker_failed", true, false, false);
            false
        }
    };
    assert!(
        passed,
        "manual recovery host lab did not complete verified rollback"
    );
}
#[test]
fn manual_lab_report_is_bounded_fresh_line_exact_five_field_contract() {
    for result in [
        "verified_rolled_back",
        "pending",
        "refused",
        "failed",
        "timed_out",
        "worker_failed",
        "rollback_unresolved",
    ] {
        let line = report_line(result, true, false, false);
        assert!(line.starts_with('\n') && line.ends_with('\n'));
        assert!(line.len() < 256);
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(value["schema"], 1);
        assert_eq!(value["result"], result);
        assert_eq!(value["worker_terminal"], true);
        assert_eq!(value["verified"], false);
        assert_eq!(value["rollback_verified"], false);
    }
}
#[test]
fn manual_lab_host_selection_is_exact_and_explicit() {
    assert!(host_selection(Some("owned-target-restore-only")));
    for marker in [
        None,
        Some(""),
        Some("owned-target-restore-only "),
        Some("arbitrary"),
    ] {
        assert!(!host_selection(marker));
    }
}
#[test]
fn manual_lab_host_guard_is_opened_canonical_absent_and_native_machine_only() {
    let native_shape = MachineIdentity {
        id: MachineId::observed(MachineIdSource::Tpm, vec![0x55; 27]).unwrap(),
    };
    assert!(host_guard(
        TuningStoreAuthority::Canonical,
        true,
        &Ok(None),
        &native_shape
    ));
    for (authority, opened, first, machine) in [
        (
            TuningStoreAuthority::IsolatedTest,
            true,
            Ok(None),
            native_shape.clone(),
        ),
        (
            TuningStoreAuthority::Canonical,
            false,
            Ok(None),
            native_shape.clone(),
        ),
        (
            TuningStoreAuthority::Canonical,
            true,
            Ok(Some(Vec::new())),
            native_shape.clone(),
        ),
        (
            TuningStoreAuthority::Canonical,
            true,
            Ok(Some(b"unknown".to_vec())),
            native_shape.clone(),
        ),
        (
            TuningStoreAuthority::Canonical,
            true,
            Err(TuningNativeError::Poisoned),
            native_shape.clone(),
        ),
        (
            TuningStoreAuthority::Canonical,
            true,
            Ok(None),
            super::machine(),
        ),
    ] {
        assert!(!host_guard(authority, opened, &first, &machine));
    }
}
#[test]
fn manual_lab_exhausted_budget_refuses_before_durable_intent_and_physical_step() {
    let root = TuningTemporaryDirectory::new().unwrap();
    drop(TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap());
    let mut native = TuningStore::open_test_directory(root.path(), MAX_JOURNAL_BYTES).unwrap();
    native.lab_arm_manual_recovery_rollback().unwrap();
    for revision in 1..=5 {
        let document = JournalDocument::capture(
            machine(),
            revision,
            JournalStamp::Recovery,
            JournalPhase::Recovery,
            &CoordinatorSnapshot::default(),
        )
        .unwrap();
        native.publish(&document.to_bytes().unwrap()).unwrap();
    }
    let before = native.read().unwrap();
    let store = Rc::new(RefCell::new(native));
    let mut target = os_api::tuning::TuningLabScalarTarget::new().unwrap();
    let writes = Arc::new(AtomicUsize::new(0));
    let mut catalog = Catalog::default();
    let module = native_module(&target, false, writes.clone()).unwrap();
    let config = module
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    catalog.register(module).unwrap();
    let operation = catalog
        .prepare(
            &config,
            TargetIdentity::Machine(machine()),
            ActivationMode::Manual,
        )
        .unwrap();
    let mut session = authority(
        NativeJournal {
            store: store.clone(),
            publications: Rc::new(RefCell::new(0)),
            machine: None,
        },
        machine(),
    );
    let result = BudgetControl {
        base: RecoveryControl {
            authority: &mut session,
        },
        store: store.clone(),
    }
    .before_write(&WriteIntent {
        operation: &operation,
        step: 0,
        ownership: CoordinatorSnapshot::default(),
    });
    assert!(matches!(
        result,
        Err(GateError::Rejected("manual lab budget"))
    ));
    assert_eq!(store.borrow_mut().read().unwrap(), before);
    assert_eq!(target.read().unwrap(), 7);
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    drop(operation);
    drop(catalog);
    drop(session);
    target.cleanup().unwrap();
    store.borrow_mut().lab_rollback().unwrap();
    drop(store);
    root.cleanup().unwrap();
}

fn acquire_native(
    store: Rc<RefCell<TuningStore>>,
    machine: MachineIdentity,
    native_machine: Option<os_api::tuning::TuningMachineIdentity>,
    count: Rc<RefCell<usize>>,
    target: &mut os_api::tuning::TuningLabScalarTarget,
) -> Result<(), ()> {
    let mut session = authority(
        NativeJournal {
            store: store.clone(),
            publications: count,
            machine: native_machine,
        },
        machine.clone(),
    );
    let writes = Arc::new(AtomicUsize::new(0));
    let module = native_module(target, false, writes.clone())?;
    let config = module
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .map_err(|_| ())?;
    let mut catalog = Catalog::default();
    catalog.register(module).map_err(|_| ())?;
    let mut coordinator = Coordinator::new(catalog);
    let mut control = BudgetControl {
        base: RecoveryControl {
            authority: &mut session,
        },
        store: store.clone(),
    };
    let result = coordinator.handle(
        Command::Acquire {
            owner: OwnerId::new(OWNER).unwrap(),
            binding: binding(),
            config,
            target: TargetIdentity::Machine(machine.clone()),
            mode: ActivationMode::Manual,
        },
        &mut control,
    );
    if result.error.is_some() || target.read() != Ok(12) || writes.load(Ordering::SeqCst) != 1 {
        return Err(());
    }
    manual_guard(&disk_document(&store, &machine)?, &intent(machine))?;
    drop(coordinator);
    drop(session);
    Ok(())
}
