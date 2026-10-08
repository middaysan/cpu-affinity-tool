//! Saved fixture input exercises the common worker protocol without native proof.
use super::*;
use crate::app::features::tuning::{
    configured_source::SavedConfiguredSource,
    platform::{catalog::Catalog, contract::*, journal::*, writer::ScopeEvidence},
    testing::scalar::{self, ScalarConfig, ScalarModule},
};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::PathBuf,
    rc::Rc,
};

const WAIT: Duration = Duration::from_secs(5);

// The public headless tests enter the existing worker with independently saved
// ordinary fixture bytes. This does not construct native scope or a receipt.
pub(super) fn start_saved_fixture(
    limits: RunnerLimits,
    catalog: Catalog,
    state: PathBuf,
    journal: PathBuf,
) -> Result<Runner, StartError> {
    struct DiskStore {
        path: PathBuf,
        _local: Rc<()>,
    }
    fn io_error(_: std::io::Error) -> JournalIoError {
        JournalIoError {
            kind: JournalIoKind::Io,
            operation: "headless fixture journal",
            native_code: None,
        }
    }
    impl JournalStore for DiskStore {
        fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
            match std::fs::File::open(&self.path) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(4 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(io_error)?;
                    Ok(StoredJournal::Bytes(bytes))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoredJournal::Absent),
                Err(e) => Err(io_error(e)),
            }
        }
        fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.path)
                .map_err(io_error)?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(io_error)
        }
        fn validate_authority(&mut self) -> Result<(), JournalIoError> {
            Ok(())
        }
    }
    struct Lifetime;
    impl super::super::recovery::RecoveryResolver for Lifetime {
        fn owner(
            &mut self,
            _: &JournalOwner,
            _: &JournalSource,
        ) -> super::super::recovery::OwnerAttachment {
            super::super::recovery::OwnerAttachment::Unknown
        }
        fn resource(&mut self, _: &ResourceKey) -> super::super::recovery::ResourceLifetime {
            super::super::recovery::ResourceLifetime::VerifiedLive
        }
    }
    fn read_state(path: &std::path::Path) -> Result<Arc<[u8]>, GateError> {
        let mut bytes = Vec::new();
        let file =
            std::fs::File::open(path).map_err(|_| GateError::Rejected("fixture source read"))?;
        file.take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| GateError::Rejected("fixture source read"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(GateError::Rejected("fixture source bound"));
        }
        Ok(Arc::from(bytes))
    }
    Runner::start_test_writer(limits, move || {
        let saved = read_state(&state).unwrap();
        let origin = super::super::configured_intent::saved_configuration(&saved)
            .map(|config| ConfigurationOrigin::fixture_lineage(config.lineage))
            .unwrap_or_else(ConfigurationOrigin::fixture);
        let source = SavedConfiguredSource::from_fixture(&saved, origin.clone(), &catalog);
        let mut scope = ScopeEvidence::fixture_saved(
            origin,
            || Ok(machine()),
            saved,
            move || read_state(&state),
        );
        if let ScopeEvidence::Fixture { lifetime, .. } = &mut scope {
            *lifetime = Some(Box::new(Lifetime));
        }
        super::super::writer::bootstrap_configured(
            catalog,
            machine(),
            Box::new(DiskStore {
                path: journal,
                _local: Rc::new(()),
            }),
            Arc::new(source),
            scope,
        )
    })
}
#[test]
fn p2b_queued_abort_cannot_be_overwritten_by_a_late_grant() {
    let state = Mutex::new(ReservationState {
        phase: SavePhase::Queued,
        saved: None,
        acknowledgements: None,
    });
    assert_eq!(state.lock().unwrap().phase, SavePhase::Queued);
    // The caller queues Abort after the worker's earlier Queued observation.
    // This is the same locked state update used by the actual worker grant.
    state.lock().unwrap().phase = SavePhase::Aborting;
    state.lock().unwrap().grant();
    assert_eq!(state.lock().unwrap().phase, SavePhase::Aborting);
}

struct SavedFile(PathBuf);
impl SavedFile {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = PathBuf::from("target").join(format!(
            "p2b-saved-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        Self(path)
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        std::fs::File::open(&self.0)
            .unwrap()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(bytes.len() <= 16 * 1024 * 1024);
        bytes
    }
    fn replace(&self, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.0)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }
}
impl Drop for SavedFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}
#[derive(Default)]
struct Memory {
    bytes: Option<Vec<u8>>,
    created: Option<thread::ThreadId>,
    dropped: Option<thread::ThreadId>,
}
struct Store {
    memory: Arc<Mutex<Memory>>,
    _local: Rc<()>,
}
impl Store {
    fn open(memory: Arc<Mutex<Memory>>) -> Self {
        memory.lock().unwrap().created = Some(thread::current().id());
        Self {
            memory,
            _local: Rc::new(()),
        }
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        self.memory.lock().unwrap().dropped = Some(thread::current().id());
    }
}
impl JournalStore for Store {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
        Ok(self
            .memory
            .lock()
            .unwrap()
            .bytes
            .clone()
            .map_or(StoredJournal::Absent, StoredJournal::Bytes))
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
        self.memory.lock().unwrap().bytes = Some(bytes.to_vec());
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
fn module() -> ScalarModule {
    let mut d = scalar::descriptor();
    d.resources[0].lifetime = ResourceLifetimeRequirement::PersistentMachine;
    ScalarModule::with_descriptor(d)
}
fn bytes(module: &ScalarModule, theme: usize) -> Vec<u8> {
    let config = module.config_envelope(&ScalarConfig { value: 12 }).unwrap();
    let binding = ActivationBinding::Manual(ConfiguredIntent {
        profile: ProfileId::new(1).unwrap(),
        generation: ConfigGeneration::new(1).unwrap(),
    });
    let state = crate::app::models::AppStateStorage::test_fixture(|s| s.theme_index = theme);
    let mut value = serde_json::to_value(state).unwrap();
    value["tuning"] = json!({"version":1,"lineage":vec![1;32],
        "profiles":[{"version":1,"id":1,"name":"P","modules":[{"feature":config.feature,
        "enabled":true,"mode":"Manual","config":config}]}],"associations":[],
        "allocations":{"next_profile":2,"next_owner":2,"next_config_generation":2},
        "activations":[{"version":1,"owner":1,"binding":binding,"config":config,
            "target":TargetIdentity::Machine(machine()),"mode":"Manual"}],"withdrawals":[]});
    serde_json::to_vec(&value).unwrap()
}
fn acquire(module: &ScalarModule) -> Command {
    Command::Acquire {
        owner: OwnerId::new(1).unwrap(),
        binding: ActivationBinding::Manual(ConfiguredIntent {
            profile: ProfileId::new(1).unwrap(),
            generation: ConfigGeneration::new(1).unwrap(),
        }),
        config: module.config_envelope(&ScalarConfig { value: 12 }).unwrap(),
        target: TargetIdentity::Machine(machine()),
        mode: ActivationMode::Manual,
    }
}
fn edited_bytes(module: &ScalarModule) -> Vec<u8> {
    let mut state: serde_json::Value = serde_json::from_slice(&bytes(module, 0)).unwrap();
    let old = state["tuning"]["activations"][0].clone();
    let mut new = old.clone();
    new["binding"]["Manual"]["generation"] = json!(2);
    new["config"] =
        serde_json::to_value(module.config_envelope(&ScalarConfig { value: 13 }).unwrap()).unwrap();
    state["tuning"]["version"] = json!(2);
    state["tuning"]["profiles"][0]["modules"][0]["config"] = new["config"].clone();
    state["tuning"]["activations"] = json!([]);
    state["tuning"]["transitions"] =
        json!([{"version":1,"owner":1,"generation":2,"old":old,"new":new}]);
    state["tuning"]["allocations"]["next_config_generation"] = json!(3);
    serde_json::to_vec(&state).unwrap()
}
fn start(module: &ScalarModule) -> (Runner, SavedFile, Arc<Mutex<Memory>>) {
    start_with(
        module,
        RunnerLimits::default(),
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
        false,
    )
}
fn start_with(
    module: &ScalarModule,
    limits: RunnerLimits,
    eligibility: Arc<AtomicBool>,
    panic_check: Arc<AtomicBool>,
    staged: bool,
) -> (Runner, SavedFile, Arc<Mutex<Memory>>) {
    let mut catalog = Catalog::default();
    catalog.register(module.clone()).unwrap();
    let file = SavedFile::new(&bytes(module, 0));
    let saved = file.bytes();
    let observed_path = file.0.clone();
    let memory = Arc::new(Mutex::new(Memory::default()));
    let retained = memory.clone();
    let runner = Runner::start_test_writer(limits, move || {
        let source =
            SavedConfiguredSource::from_fixture(&saved, ConfigurationOrigin::fixture(), &catalog);
        let (coordinator, mut authority) = super::super::writer::bootstrap_configured(
            catalog,
            machine(),
            Box::new(Store::open(retained)),
            Arc::new(source),
            ScopeEvidence::fixture_saved(
                ConfigurationOrigin::fixture(),
                move || {
                    assert!(
                        !panic_check.load(Ordering::Acquire),
                        "fixture authority check panic"
                    );
                    if !eligibility.load(Ordering::Acquire) {
                        return Err(GateError::Rejected("fixture eligibility invalidated"));
                    }
                    Ok(machine())
                },
                Arc::from(saved),
                move || {
                    let mut bytes = Vec::new();
                    std::fs::File::open(&observed_path)
                        .map_err(|_| GateError::Rejected("fixture read failed"))?
                        .take(16 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| GateError::Rejected("fixture read failed"))?;
                    if bytes.len() > 16 * 1024 * 1024 {
                        return Err(GateError::Rejected("fixture read bound"));
                    }
                    Ok(Arc::from(bytes))
                },
            ),
        );
        if staged {
            authority.stage_configuration_for_test();
        }
        (coordinator, authority)
    })
    .unwrap();
    (runner, file, memory)
}

#[test]
fn p2b_reservation_waits_for_real_earlier_work_and_closes_new_admission() {
    let module = module();
    let block = super::tests::Block::once();
    module.set_execute_hook(block.hook.clone());
    let (mut runner, _file, memory) = start(&module);
    let client = runner.client();
    let earlier = client.submit(acquire(&module), 1).unwrap();
    block.entered();
    let reserved = client.reserve_configuration();
    if let Ok(ticket) = &reserved {
        assert_eq!(ticket.wait(Duration::ZERO), Err(ReservationError::TimedOut));
    }
    block.release();
    let mut reserved = reserved.unwrap();
    assert_eq!(
        client.submit(acquire(&module), 1).err(),
        Some(SubmitError::Reserved)
    );
    assert_eq!(
        client.reserve_configuration().err(),
        Some(ReservationError::Busy)
    );
    earlier.wait(WAIT).unwrap();
    assert_eq!(reserved.wait(WAIT), Ok(SavePhase::Granted));
    assert!(memory.lock().unwrap().dropped.is_none());
    reserved.abort(WAIT).unwrap();
    assert_eq!(reserved.phase(), SavePhase::Aborted);
    assert!(client
        .submit(acquire(&module), 1)
        .unwrap()
        .wait(WAIT)
        .is_some());
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
    let memory = memory.lock().unwrap();
    assert_eq!(memory.created, memory.dropped);
}
#[test]
fn p2b_shutdown_settles_pre_start_reservation_when_stop_queue_is_full() {
    let module = module();
    let block = super::tests::Block::once();
    module.set_observe_hook(block.hook.clone());
    let (mut runner, _file, memory) = start_with(
        &module,
        RunnerLimits {
            queue_capacity: 1,
            generation_owners: 256,
        },
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    let client = runner.client();
    let earlier = client.submit(super::tests::observe(), 1).unwrap();
    block.entered();
    let mut reserved = client.reserve_configuration().unwrap();
    assert_eq!(reserved.phase(), SavePhase::Queued);
    assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
    assert_eq!(
        reserved.begin_publication(Duration::ZERO),
        Err(ReservationError::Closed)
    );
    assert_eq!(
        reserved.abort(Duration::ZERO),
        Err(ReservationError::Closed)
    );
    block.release();
    assert!(matches!(
        earlier.wait(WAIT).unwrap().outcome,
        TerminalOutcome::Completed(_)
    ));
    // Keeping the exact reservation continuation alive cannot pin shutdown.
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
    assert_eq!(reserved.phase(), SavePhase::Aborted);
    assert_eq!(module.writes(), 0);
    let memory = memory.lock().unwrap();
    assert_eq!(memory.created, memory.dropped);
}
#[test]
fn p2b_matching_saved_handoff_installs_on_same_worker_after_start_ack() {
    let module = module();
    let (mut runner, file, memory) = start(&module);
    let client = runner.client();
    client
        .submit(acquire(&module), 1)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    let old_journal = memory.lock().unwrap().bytes.clone();
    let mut reserved = client.reserve_configuration().unwrap();
    assert_eq!(reserved.wait(WAIT), Ok(SavePhase::Granted));
    assert_eq!(
        reserved.install_fixture(&[], ConfigurationOrigin::fixture(), WAIT),
        Err(ReservationError::InvalidPhase)
    );
    reserved.begin_publication(WAIT).unwrap();
    file.replace(&bytes(&module, 2));
    reserved
        .install_fixture(&file.bytes(), ConfigurationOrigin::fixture(), WAIT)
        .unwrap();
    assert_eq!(reserved.phase(), SavePhase::Installed);
    assert_eq!(memory.lock().unwrap().bytes, old_journal);
    assert_eq!(module.value(), 12);
    assert!(client
        .submit(acquire(&module), 1)
        .unwrap()
        .wait(WAIT)
        .is_some());
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
    let memory = memory.lock().unwrap();
    assert_eq!(memory.created, memory.dropped);
}
#[test]
fn p2b_saved_same_owner_edit_preserves_first_baseline_through_live_handoff() {
    let module = module();
    let (mut runner, file, _) = start(&module);
    let client = runner.client();
    client
        .submit(acquire(&module), 1)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    let first = runner.snapshot().coordinator.resources[0].baseline.clone();
    let mut reserved = client.reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    reserved.begin_publication(WAIT).unwrap();
    file.replace(&edited_bytes(&module));
    reserved
        .install_fixture(&file.bytes(), ConfigurationOrigin::fixture(), WAIT)
        .unwrap();
    let mut requested = acquire(&module);
    if let Command::Acquire {
        binding, config, ..
    } = &mut requested
    {
        *binding = ActivationBinding::Manual(ConfiguredIntent {
            profile: ProfileId::new(1).unwrap(),
            generation: ConfigGeneration::new(2).unwrap(),
        });
        *config = module.config_envelope(&ScalarConfig { value: 13 }).unwrap();
    }
    let result = client.submit(requested, 2).unwrap().wait(WAIT).unwrap();
    assert!(matches!(result.outcome, TerminalOutcome::Completed(result) if result.error.is_none()));
    assert_eq!(module.value(), 13);
    let snapshot = runner.snapshot().coordinator;
    assert_eq!(snapshot.owners.len(), 1);
    assert_eq!(snapshot.resources[0].baseline, first);
    assert_eq!(snapshot.owners[0].owner, OwnerId::new(1).unwrap());
    assert_eq!(
        client.submit(acquire(&module), 1).err(),
        Some(SubmitError::StaleGeneration { current: 2 })
    );
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
}

#[test]
fn p2b_saved_update_recovers_each_actual_durable_boundary_and_original_baseline() {
    struct Crash {
        boundary: u8,
        document: Option<JournalDocument>,
    }
    impl ExecutionControl for Crash {
        fn cancellation_requested(&self) -> bool {
            false
        }
        fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
            self.document = Some(
                JournalDocument::capture(
                    machine(),
                    4,
                    JournalStamp::Recovery,
                    JournalPhase::BeforeWrite {
                        step: intent.step as u32,
                        effect: intent.operation.effect(),
                    },
                    &intent.ownership,
                )
                .unwrap(),
            );
            if self.boundary == 1 {
                panic!("fixture stop at durable new intent");
            }
            Ok(())
        }
        fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
            if self.boundary == 3 {
                self.document = Some(
                    JournalDocument::capture(
                        machine(),
                        5,
                        JournalStamp::Recovery,
                        JournalPhase::VerifiedWrite {
                            step: intent.step as u32,
                            effect: intent.operation.effect(),
                        },
                        &intent.ownership,
                    )
                    .unwrap(),
                );
            }
            panic!("fixture stop after actual new target write");
        }
        fn publish(&mut self, _: CoordinatorSnapshot) {}
        fn commit_transition(&mut self, _: &CoordinatorSnapshot) -> Result<(), GateError> {
            Ok(())
        }
    }
    for (boundary, observed_original) in [
        (0, false),
        (1, false),
        (2, false),
        (3, false),
        (0, true),
        (1, true),
    ] {
        let module = module();
        let (mut original_runner, file, memory) = start(&module);
        original_runner
            .client()
            .submit(acquire(&module), 1)
            .unwrap()
            .wait(WAIT)
            .unwrap();
        let snapshot = original_runner.snapshot().coordinator;
        let first = snapshot.resources[0].baseline.clone();
        let old = JournalDocument::capture(
            machine(),
            3,
            JournalStamp::Recovery,
            JournalPhase::Transition,
            &snapshot,
        )
        .unwrap();
        assert_eq!(original_runner.shutdown(WAIT), ShutdownResult::Complete);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        let document = if boundary == 0 {
            old
        } else {
            let mut coordinator = Coordinator::recover(catalog.clone(), &old).unwrap();
            let mut command = acquire(&module);
            if let Command::Acquire {
                binding, config, ..
            } = &mut command
            {
                *binding = ActivationBinding::Manual(ConfiguredIntent {
                    profile: ProfileId::new(1).unwrap(),
                    generation: ConfigGeneration::new(2).unwrap(),
                });
                *config = module.config_envelope(&ScalarConfig { value: 13 }).unwrap();
            }
            let mut crash = Crash {
                boundary,
                document: None,
            };
            assert!(
                catch_unwind(AssertUnwindSafe(|| coordinator.handle(command, &mut crash))).is_err()
            );
            if boundary >= 2 {
                assert_eq!(module.value(), 13);
            }
            crash.document.unwrap()
        };
        if observed_original {
            module.external_change(7);
        }
        file.replace(&edited_bytes(&module));
        memory.lock().unwrap().bytes = Some(document.to_bytes().unwrap());
        let source = SavedConfiguredSource::from_fixture(
            &file.bytes(),
            ConfigurationOrigin::fixture(),
            &catalog,
        );
        let (recovered, authority) = super::super::writer::bootstrap_configured(
            catalog,
            machine(),
            Box::new(Store::open(memory.clone())),
            Arc::new(source),
            ScopeEvidence::fixture(|| Ok(machine())),
        );
        assert!(
            !matches!(authority.status(), WriterStatus::Blocked(_)),
            "boundary {boundary}, original {observed_original}"
        );
        assert_eq!(
            module.value(),
            13,
            "boundary {boundary}, original {observed_original}"
        );
        let snapshot = recovered.snapshot();
        assert_eq!(snapshot.resources[0].baseline, first);
        assert_eq!(
            snapshot.resources[0].confirmed,
            snapshot.resources[0].desired
        );
        assert!(snapshot.resources[0].possible.is_empty());
        assert_eq!(
            snapshot.owners[0]
                .binding
                .configured()
                .unwrap()
                .generation
                .get(),
            2
        );
    }
}
#[test]
fn p2b_changed_owned_revision_cannot_abort_back_to_old_input() {
    let module = module();
    let (mut runner, file, memory) = start(&module);
    let client = runner.client();
    client
        .submit(acquire(&module), 1)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    let journal = memory.lock().unwrap().bytes.clone();
    let mut reserved = client.reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    file.replace(&bytes(&module, 2));
    assert_eq!(reserved.abort(WAIT), Err(ReservationError::Refused));
    assert_eq!(reserved.phase(), SavePhase::Uncertain);
    assert_eq!(memory.lock().unwrap().bytes, journal);
    assert_eq!(module.value(), 12);
    assert_eq!(
        client.submit(acquire(&module), 1).err(),
        Some(SubmitError::Reserved)
    );
    drop(reserved);
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
}
#[test]
fn p2b_saved_handoff_must_match_actual_owned_bytes() {
    let module = module();
    let (mut runner, file, memory) = start(&module);
    let mut reserved = runner.client().reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    reserved.begin_publication(WAIT).unwrap();
    file.replace(&bytes(&module, 1));
    let foreign = SavedFile::new(&bytes(&module, 2));
    assert_eq!(
        reserved.install_fixture(&foreign.bytes(), ConfigurationOrigin::fixture(), WAIT),
        Err(ReservationError::Refused)
    );
    assert_eq!(reserved.phase(), SavePhase::Uncertain);
    assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
    assert!(memory.lock().unwrap().dropped.is_none());
    let state = reserved.data.state.lock().unwrap();
    let SavedRevision::Fixture {
        bytes: retained, ..
    } = state.saved.as_deref().unwrap()
    else {
        panic!("fixture evidence")
    };
    assert_eq!(&**retained, foreign.bytes());
}
#[test]
fn p2b_foreign_unknown_and_unsupported_handoffs_keep_saved_and_original_journal() {
    for case in 0..3 {
        let module = module();
        let (mut runner, file, memory) = start(&module);
        let client = runner.client();
        client
            .submit(acquire(&module), 1)
            .unwrap()
            .wait(WAIT)
            .unwrap();
        let journal = memory.lock().unwrap().bytes.clone();
        let mut reserved = client.reserve_configuration().unwrap();
        reserved.wait(WAIT).unwrap();
        reserved.begin_publication(WAIT).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes(&module, 2)).unwrap();
        let mut origin = ConfigurationOrigin::fixture();
        if case == 0 {
            origin = origin.fixture_changed("lineage");
        }
        if case == 1 {
            value["tuning"]["version"] = json!(99);
        }
        if case == 2 {
            value["tuning"]["profiles"][0]["modules"][0]["config"]["version"] = json!(99);
            value["tuning"]["activations"][0]["config"]["version"] = json!(99);
        }
        file.replace(&serde_json::to_vec(&value).unwrap());
        assert_eq!(
            reserved.install_fixture(&file.bytes(), origin, WAIT),
            Err(ReservationError::Refused)
        );
        assert_eq!(memory.lock().unwrap().bytes, journal);
        assert_eq!(module.value(), 12);
        assert_eq!(module.writes(), 1);
        assert_eq!(reserved.phase(), SavePhase::Uncertain);
        assert!(reserved.data.state.lock().unwrap().saved.is_some());
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        assert!(memory.lock().unwrap().dropped.is_none());
    }
}
#[test]
fn p2b_queue_pressure_and_stale_reservation_do_not_consume_ids_or_old_work() {
    let module = module();
    let block = super::tests::Block::once();
    module.set_execute_hook(block.hook.clone());
    let (mut runner, _file, _) = start_with(
        &module,
        RunnerLimits {
            queue_capacity: 1,
            generation_owners: 256,
        },
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    let client = runner.client();
    let earlier = client.submit(acquire(&module), 1).unwrap();
    block.entered();
    let queued = client.submit(super::tests::observe(), 1).unwrap();
    assert_eq!(
        client.reserve_configuration().err(),
        Some(ReservationError::QueueFull)
    );
    assert_eq!(
        client.shared.admission.lock().unwrap().next_operation,
        Some(3)
    );
    block.release();
    earlier.wait(WAIT).unwrap();
    queued.wait(WAIT).unwrap();
    let mut first = client.reserve_configuration().unwrap();
    assert_eq!(first.data.id.get(), 3);
    first.wait(WAIT).unwrap();
    first.abort(WAIT).unwrap();
    let mut next = client.reserve_configuration().unwrap();
    next.wait(WAIT).unwrap();
    assert_eq!(first.begin_publication(WAIT), Err(ReservationError::Stale));
    assert_eq!(next.phase(), SavePhase::Granted);
    next.abort(WAIT).unwrap();
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
}
#[test]
fn p2b_reservation_retains_generation_retirement_state() {
    let module = module();
    let (mut runner, _file, _) = start(&module);
    let client = runner.client();
    let mut reserved = client.reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    assert_eq!(
        client.retire_generation_owner(OwnerId::new(999).unwrap(), 1),
        Err(RetirementError::Busy)
    );
    reserved.abort(WAIT).unwrap();
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
}
#[test]
fn p2b_independent_invalidation_during_reservation_refuses_resume() {
    let module = module();
    let eligibility = Arc::new(AtomicBool::new(true));
    let (mut runner, _file, memory) = start_with(
        &module,
        RunnerLimits::default(),
        eligibility.clone(),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    let client = runner.client();
    let mut reserved = client.reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    eligibility.store(false, Ordering::Release);
    assert_eq!(reserved.abort(WAIT), Err(ReservationError::Refused));
    assert_eq!(
        client.submit(acquire(&module), 1).err(),
        Some(SubmitError::Reserved)
    );
    assert_eq!(module.writes(), 0);
    drop(reserved);
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
    assert!(memory.lock().unwrap().dropped.is_some());
}
#[test]
fn p2b_complete_source_does_not_open_staged_admission() {
    let module = module();
    let (mut runner, file, _) = start_with(
        &module,
        RunnerLimits::default(),
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
        true,
    );
    let client = runner.client();
    let mut reserved = client.reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    reserved.begin_publication(WAIT).unwrap();
    file.replace(&bytes(&module, 1));
    reserved
        .install_fixture(&file.bytes(), ConfigurationOrigin::fixture(), WAIT)
        .unwrap();
    let completed = client
        .submit(acquire(&module), 1)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    assert_eq!(module.writes(), 0);
    assert_eq!(module.reads(), 0);
    assert!(
        matches!(completed.outcome, TerminalOutcome::Completed(result) if result.error.is_some())
    );
    assert_eq!(runner.snapshot().writer, WriterStatus::ConfigurationStaged);
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
}
#[test]
fn p2b_started_drop_and_panic_retain_actual_worker_context() {
    for panic in [false, true] {
        let module = module();
        let panic_check = Arc::new(AtomicBool::new(false));
        let (mut runner, file, memory) = start_with(
            &module,
            RunnerLimits::default(),
            Arc::new(AtomicBool::new(true)),
            panic_check.clone(),
            false,
        );
        let mut reserved = runner.client().reserve_configuration().unwrap();
        reserved.wait(WAIT).unwrap();
        reserved.begin_publication(WAIT).unwrap();
        if panic {
            file.replace(&bytes(&module, 2));
            panic_check.store(true, Ordering::Release);
            assert_eq!(
                reserved.install_fixture(&file.bytes(), ConfigurationOrigin::fixture(), WAIT),
                Err(ReservationError::Refused)
            );
            assert_eq!(runner.snapshot().phase, RunnerPhase::Faulted);
        }
        drop(reserved);
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        assert!(memory.lock().unwrap().dropped.is_none());
        assert_eq!(module.writes(), 0);
    }
    // These deliberately unresolved fixture-only stacks live until this test
    // process exits. There is no native handle/actor or replacement worker.
}
#[test]
fn p2b_failed_publication_without_saved_proof_is_explicit_uncertainty() {
    let module = module();
    let (mut runner, _file, memory) = start(&module);
    let mut reserved = runner.client().reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    reserved.begin_publication(WAIT).unwrap();
    reserved.publication_uncertain().unwrap();
    assert_eq!(reserved.phase(), SavePhase::Uncertain);
    assert_eq!(
        runner.client().submit(acquire(&module), 1).err(),
        Some(SubmitError::Reserved)
    );
    assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
    assert!(memory.lock().unwrap().dropped.is_none());
}
#[test]
fn p2b_rejected_handoff_blocked_status_precedes_staged_status() {
    let module = module();
    let (mut runner, file, memory) = start_with(
        &module,
        RunnerLimits::default(),
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
        true,
    );
    let mut reserved = runner.client().reserve_configuration().unwrap();
    reserved.wait(WAIT).unwrap();
    reserved.begin_publication(WAIT).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes(&module, 1)).unwrap();
    value["tuning"]["version"] = json!(99);
    file.replace(&serde_json::to_vec(&value).unwrap());
    assert_eq!(
        reserved.install_fixture(&file.bytes(), ConfigurationOrigin::fixture(), WAIT),
        Err(ReservationError::Refused)
    );
    assert!(matches!(runner.snapshot().writer, WriterStatus::Blocked(_)));
    assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
    assert!(memory.lock().unwrap().dropped.is_none());
}
#[test]
fn p2b_started_publication_timeout_retains_same_context_until_matching_handoff() {
    let module = module();
    let (mut runner, file, memory) = start(&module);
    let mut reserved = runner.client().reserve_configuration().unwrap();
    assert_eq!(reserved.wait(WAIT), Ok(SavePhase::Granted));
    reserved.begin_publication(WAIT).unwrap();
    assert_eq!(reserved.abort(WAIT), Err(ReservationError::InvalidPhase));
    assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
    assert!(memory.lock().unwrap().dropped.is_none());
    file.replace(&bytes(&module, 1));
    reserved
        .install_fixture(&file.bytes(), ConfigurationOrigin::fixture(), WAIT)
        .unwrap();
    drop(reserved);
    assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
    let memory = memory.lock().unwrap();
    assert_eq!(memory.created, memory.dropped);
}
