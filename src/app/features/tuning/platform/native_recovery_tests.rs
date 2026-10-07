//! Nonshipping forced-process interruption with real isolated metadata files.
//! The resource/game binding and resolver are fixtures, never native admission.
use super::{
    catalog::{Catalog, Module},
    codec::{ModuleEnvelope, VersionedCodec},
    contract::*,
    coordinator::{
        Command, Coordinator, CoordinatorSnapshot, ExecutionControl, GateError, Outcome,
        WriteIntent,
    },
    journal::{
        DurableSession, JournalDocument, JournalIoError, JournalIoKind, JournalLoad, JournalOwner,
        JournalPhase, JournalSource, JournalStamp, JournalStore, StoredJournal, MAX_JOURNAL_BYTES,
    },
    recovery::{OwnerAttachment, RecoveryPlan, RecoveryResolver, ResourceLifetime},
    runner::{Runner, RunnerLimits, ShutdownResult},
    writer::WorkerAuthority,
};
use crate::app::features::tuning::testing::scalar::{
    ScalarConfig, ScalarModule, ScalarSnapshot, ScalarStep,
};
use os_api::tuning::{
    TuningNativeError, TuningStore, TuningStoreAuthority, TuningTemporaryDirectory,
};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CHILD_TEST: &str =
    "app::features::tuning::platform::native_recovery_tests::isolated_native_child";
const ROOT_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_ROOT";
const TARGET_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_TARGET";
const ROLE_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_ROLE";
const PAUSED: &str = "NATIVE_FIXTURE_PAUSED";
const TIMED_OUT: &str = "NATIVE_FIXTURE_TIMED_OUT";
const RECOVERED: &str = "NATIVE_FIXTURE_RECOVERED";
const PROTOCOL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PROTOCOL_LINES: usize = 64;
const MAX_PROTOCOL_LINE_BYTES: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopPoint {
    Intent,
    Write,
    Progress,
}
impl StopPoint {
    fn role(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Write => "write",
            Self::Progress => "progress",
        }
    }
    fn parse(role: &str) -> Self {
        match role {
            "intent" => Self::Intent,
            "write" => Self::Write,
            "progress" => Self::Progress,
            _ => panic!("unknown isolated writer role"),
        }
    }
}
fn machine() -> MachineIdentity {
    MachineIdentity {
        id: MachineId::new([0x42; 16]).unwrap(),
    }
}
fn read_target(path: &Path) -> i32 {
    i32::from_le_bytes(std::fs::read(path).unwrap().try_into().unwrap())
}
fn write_target(path: &Path, value: i32) {
    // Only the parent-owned, existing four-byte fixture is writable. This does
    // not create a setting path or replace/remove a directory.
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    assert_eq!(file.metadata().unwrap().len(), 4);
    file.write_all(&value.to_le_bytes()).unwrap();
    file.sync_all().unwrap();
}
fn read_document(root: &Path) -> JournalDocument {
    let mut store = TuningStore::open_test_directory(root, MAX_JOURNAL_BYTES).unwrap();
    let bytes = store
        .read()
        .unwrap()
        .expect("interrupted durable record must survive");
    let JournalLoad::Known(document) = JournalDocument::load(bytes, &machine()) else {
        panic!("known fixture journal expected");
    };
    document
}

struct ProbeChild {
    child: Child,
    messages: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}
impl ProbeChild {
    fn spawn(root: &Path, target: &Path, role: &str) -> Self {
        let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(ROOT_ENV, root)
            .env(TARGET_ENV, target)
            .env(ROLE_ENV, role)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::sync_channel(MAX_PROTOCOL_LINES);
        let reader = thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            for _ in 0..MAX_PROTOCOL_LINES {
                let mut line = String::new();
                let result = (&mut input)
                    .take((MAX_PROTOCOL_LINE_BYTES + 1) as u64)
                    .read_line(&mut line);
                if !matches!(result, Ok(1..))
                    || line.len() > MAX_PROTOCOL_LINE_BYTES
                    || !line.ends_with('\n')
                {
                    break;
                }
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            messages,
            reader: Some(reader),
        }
    }
    fn wait_message(&self, wanted: &str) {
        // One total deadline includes every unrelated test-harness line.
        let deadline = Instant::now() + PROTOCOL_TIMEOUT;
        for _ in 0..MAX_PROTOCOL_LINES {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = self
                .messages
                .recv_timeout(remaining)
                .expect("owned child did not reach the required native boundary");
            if line.trim() == wanted {
                return;
            }
        }
        panic!("owned child protocol message limit");
    }
    fn terminate(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "child must retain its worker before forced termination"
        );
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }
}
impl Drop for ProbeChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn interruption(point: StopPoint) {
    let journal_root = TuningTemporaryDirectory::new().unwrap();
    let target_root = TuningTemporaryDirectory::new().unwrap();
    let target = target_root.path().join("scalar-native-target.bin");
    std::fs::write(&target, 7i32.to_le_bytes()).unwrap();
    let mut writer = ProbeChild::spawn(journal_root.path(), &target, point.role());
    writer.wait_message(PAUSED);
    writer.wait_message(TIMED_OUT);
    assert!(
        matches!(
            TuningStore::open_test_directory(journal_root.path(), MAX_JOURNAL_BYTES),
            Err(TuningNativeError::Busy)
        ),
        "timed-out worker must retain exact native lease"
    );
    writer.terminate();
    let expected = if point == StopPoint::Intent { 7 } else { 12 };
    assert_eq!(read_target(&target), expected);
    let interrupted = read_document(journal_root.path());
    assert!(matches!(
        interrupted.stamp,
        JournalStamp::Operation {
            config_generation: 1,
            ..
        }
    ));
    assert!(matches!(
        (&point, &interrupted.phase),
        (
            StopPoint::Progress,
            JournalPhase::VerifiedWrite { step: 0, .. }
        ) | (
            StopPoint::Intent | StopPoint::Write,
            JournalPhase::BeforeWrite { step: 0, .. }
        )
    ));
    assert_eq!(interrupted.resources.len(), 1);
    assert_eq!(
        interrupted.resources[0].baseline.bytes(),
        7i32.to_le_bytes()
    );
    if point == StopPoint::Progress {
        assert_eq!(
            interrupted.resources[0].confirmed.bytes(),
            12i32.to_le_bytes()
        );
    } else {
        assert!(interrupted.resources[0]
            .attempted
            .iter()
            .any(|token| token.bytes() == 12i32.to_le_bytes()));
    }
    // Child::wait attests only this fixture actor's termination, never a game,
    // Windows logon or boot. The recovery resolver is explicitly test-only.
    let mut recovery = ProbeChild::spawn(journal_root.path(), &target, "recover-fixture-expired");
    recovery.wait_message(RECOVERED);
    assert_eq!(read_target(&target), 7);
    assert!(matches!(
        TuningStore::open_test_directory(journal_root.path(), MAX_JOURNAL_BYTES),
        Err(TuningNativeError::Busy)
    ));
    recovery.terminate();
    let recovered = read_document(journal_root.path());
    assert_eq!(recovered.stamp, JournalStamp::Recovery);
    assert!(recovered.owners.is_empty() && recovered.resources.is_empty());
    assert!(recovered.revision > interrupted.revision);
    target_root.cleanup().unwrap();
    journal_root.cleanup().unwrap();
}

#[test]
fn forced_exit_after_intent_recovers_without_a_target_write() {
    interruption(StopPoint::Intent);
}
#[test]
fn forced_exit_after_physical_write_restores_exact_original_baseline() {
    interruption(StopPoint::Write);
}
#[test]
fn forced_exit_after_verified_progress_reloads_and_restores_original() {
    interruption(StopPoint::Progress);
}

struct PauseGate {
    reached: SyncSender<()>,
    waiting: Mutex<()>,
    wake: Condvar,
}
impl PauseGate {
    fn pause(&self) {
        self.reached.send(()).unwrap();
        // The parent owns termination. Neither elapsed time nor cancellation
        // can release this child worker's still-owned context/lease.
        let mut waiting = self.waiting.lock().unwrap();
        loop {
            waiting = self.wake.wait(waiting).unwrap();
        }
    }
}
fn announce(message: &str) {
    println!("\n{message}");
    std::io::stdout().flush().unwrap();
}

/// Retains the scalar's typed codec/plan but observes and conditionally writes
/// the parent's physical fixture. Recovery never relies on cached target state.
struct FileScalarModule {
    scalar: ScalarModule,
    target: PathBuf,
    pause_after_write: Option<Arc<PauseGate>>,
}
impl Module for FileScalarModule {
    type Config = ScalarConfig;
    type Observation = i32;
    type Snapshot = ScalarSnapshot;
    type Step = ScalarStep;
    fn descriptor(&self) -> ModuleDescriptor {
        self.scalar.descriptor()
    }
    fn config_codec(&self) -> &dyn VersionedCodec<Self::Config> {
        self.scalar.config_codec()
    }
    fn snapshot_codec(&self) -> &dyn VersionedCodec<Self::Snapshot> {
        self.scalar.snapshot_codec()
    }
    fn validate_config(&self, config: &Self::Config) -> Result<(), ModuleError> {
        self.scalar.validate_config(config)
    }
    fn snapshot_config_matches(&self, snapshot: &Self::Snapshot, config: &Self::Config) -> bool {
        self.scalar.snapshot_config_matches(snapshot, config)
    }
    fn capability(&self, target: &TargetIdentity) -> Capability {
        self.scalar.capability(target)
    }
    fn observe(&self, target: &TargetIdentity) -> Result<Self::Observation, ModuleError> {
        self.scalar.external_change(read_target(&self.target));
        self.scalar.observe(target)
    }
    fn observation_summary(&self, observation: &Self::Observation) -> ObservationSummary {
        self.scalar.observation_summary(observation)
    }
    fn observation_states(
        &self,
        target: &TargetIdentity,
        observation: &Self::Observation,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        self.scalar
            .observation_states(target, observation, resources)
    }
    fn prepare(
        &self,
        target: &TargetIdentity,
        config: &Self::Config,
        current: &Self::Observation,
    ) -> Result<Plan<Self::Snapshot, Self::Step>, ModuleError> {
        self.scalar.prepare(target, config, current)
    }
    fn snapshot_claims(
        &self,
        target: &TargetIdentity,
        snapshot: &Self::Snapshot,
    ) -> Result<Vec<ResourceClaim>, ModuleError> {
        self.scalar.snapshot_claims(target, snapshot)
    }
    fn baseline_states(
        &self,
        target: &TargetIdentity,
        snapshot: &Self::Snapshot,
        resources: &[ResourceKey],
    ) -> Result<Vec<ResourceObservation>, ModuleError> {
        self.scalar.baseline_states(target, snapshot, resources)
    }
    fn same_baseline(&self, left: &Self::Snapshot, right: &Self::Snapshot) -> bool {
        self.scalar.same_baseline(left, right)
    }
    fn classify(
        &self,
        snapshot: &Self::Snapshot,
        current: &Self::Observation,
        resources: &[ResourceKey],
    ) -> Result<StateMatch, ModuleError> {
        self.scalar.classify(snapshot, current, resources)
    }
    fn prepare_update(
        &self,
        target: &TargetIdentity,
        config: &Self::Config,
        original: &Self::Snapshot,
        current: &Self::Observation,
    ) -> Result<Plan<Self::Snapshot, Self::Step>, ModuleError> {
        self.scalar
            .prepare_update(target, config, original, current)
    }
    fn prepare_restore(
        &self,
        target: &TargetIdentity,
        original: &Self::Snapshot,
        current: &Self::Observation,
        released: &[ResourceKey],
    ) -> Result<Plan<Self::Snapshot, Self::Step>, ModuleError> {
        self.scalar
            .prepare_restore(target, original, current, released)
    }
    fn execute_step(
        &self,
        target: &TargetIdentity,
        step: &Self::Step,
    ) -> Result<Effect, ModuleError> {
        self.scalar.external_change(read_target(&self.target));
        let effect = self.scalar.execute_step(target, step)?;
        write_target(&self.target, self.scalar.value());
        if let Some(pause) = &self.pause_after_write {
            pause.pause();
        }
        Ok(effect)
    }
    fn verify_step(
        &self,
        target: &TargetIdentity,
        step: &Self::Step,
        current: &Self::Observation,
    ) -> Result<Verification, ModuleError> {
        self.scalar.verify_step(target, step, current)
    }
}

fn native_error(error: TuningNativeError) -> JournalIoError {
    let (kind, operation, native_code) = match error {
        TuningNativeError::Busy => (JournalIoKind::Busy, "fixture lease", None),
        TuningNativeError::Unsupported(operation) => (JournalIoKind::Unavailable, operation, None),
        TuningNativeError::UnsafeStore(operation) => (JournalIoKind::Unsafe, operation, None),
        TuningNativeError::Poisoned => (JournalIoKind::Unsafe, "fixture publication", None),
        TuningNativeError::Capacity { .. } => (JournalIoKind::Capacity, "fixture journal", None),
        TuningNativeError::Denied { operation, code } => {
            (JournalIoKind::Denied, operation, Some(code))
        }
        TuningNativeError::Io { operation, code } => (JournalIoKind::Io, operation, Some(code)),
    };
    JournalIoError {
        kind,
        operation,
        native_code,
    }
}
struct IsolatedStore {
    native: TuningStore,
    pause: Option<(StopPoint, Arc<PauseGate>)>,
}
impl JournalStore for IsolatedStore {
    fn read(&mut self) -> Result<StoredJournal, JournalIoError> {
        match self.native.read() {
            Ok(None) => Ok(StoredJournal::Absent),
            Ok(Some(bytes)) => Ok(StoredJournal::Bytes(bytes)),
            Err(TuningNativeError::Capacity { size, .. }) => Ok(StoredJournal::Oversized { size }),
            Err(error) => Err(native_error(error)),
        }
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), JournalIoError> {
        self.native.publish(bytes).map_err(native_error)?;
        if let Some((point, pause)) = &self.pause {
            let JournalLoad::Known(document) = JournalDocument::load(bytes.to_vec(), &machine())
            else {
                panic!("fixture publication must be typed and valid");
            };
            if matches!(
                (point, &document.phase),
                (StopPoint::Intent, JournalPhase::BeforeWrite { .. })
                    | (StopPoint::Progress, JournalPhase::VerifiedWrite { .. })
            ) {
                pause.pause();
            }
        }
        Ok(())
    }
    fn validate_authority(&mut self) -> Result<(), JournalIoError> {
        assert_eq!(self.native.authority(), TuningStoreAuthority::IsolatedTest);
        self.native.validate_authority().map_err(native_error)
    }
}

/// Parent Child::wait supplied fixture-actor expiry. This cannot be installed
/// as a production Windows game/logon/boot lifetime producer.
struct ExpiredFixtureActor;
impl RecoveryResolver for ExpiredFixtureActor {
    fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
        match &owner.binding {
            ActivationBinding::Game(binding) => OwnerAttachment::PositivelyExpired {
                owner: owner.owner,
                binding: binding.clone(),
            },
            _ => OwnerAttachment::Unknown,
        }
    }
    fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
        ResourceLifetime::VerifiedLive
    }
}
struct RecoveryControl<'a> {
    authority: &'a mut WorkerAuthority,
}
impl ExecutionControl for RecoveryControl<'_> {
    fn cancellation_requested(&self) -> bool {
        false
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.authority.commit(
            JournalStamp::Recovery,
            JournalPhase::BeforeWrite {
                step: u32::try_from(intent.step).unwrap(),
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )
    }
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.authority.commit(
            JournalStamp::Recovery,
            JournalPhase::VerifiedWrite {
                step: u32::try_from(intent.step).unwrap(),
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )
    }
    fn publish(&mut self, _: CoordinatorSnapshot) {}
    fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
        self.authority
            .commit(JournalStamp::Recovery, JournalPhase::Recovery, snapshot)
    }
}

#[test]
fn isolated_native_child() {
    let Some(role) = std::env::var_os(ROLE_ENV) else {
        return;
    };
    let role = role.to_str().unwrap().to_owned();
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
    let target = PathBuf::from(std::env::var_os(TARGET_ENV).unwrap());
    let recovering = role == "recover-fixture-expired";
    let point = (!recovering).then(|| StopPoint::parse(&role));
    let (reached, boundary) = mpsc::sync_channel(1);
    let pause = Arc::new(PauseGate {
        reached,
        waiting: Mutex::new(()),
        wake: Condvar::new(),
    });
    let worker_pause = pause.clone();
    let (ready, bootstrap) = mpsc::sync_channel::<(ModuleEnvelope, ActivationBinding)>(1);
    let mut runner = Runner::start_test_writer(RunnerLimits::default(), move || {
        // Store, exact physical observation, typed hydration and recovery all
        // occur on this serial worker with one native isolated lease retained.
        let native = TuningStore::open_test_directory(&root, MAX_JOURNAL_BYTES).unwrap();
        assert_eq!(native.authority(), TuningStoreAuthority::IsolatedTest);
        let module = FileScalarModule {
            scalar: ScalarModule::new(read_target(&target)),
            target,
            pause_after_write: (point == Some(StopPoint::Write)).then(|| worker_pause.clone()),
        };
        let config = module
            .scalar
            .config_envelope(&ScalarConfig { value: 12 })
            .unwrap();
        let mut catalog = Catalog::default();
        catalog.register(module).unwrap();
        let store: Box<dyn JournalStore> = Box::new(IsolatedStore {
            native,
            pause: point.map(|point| (point, worker_pause.clone())),
        });
        let session = DurableSession::bootstrap(store, machine());
        let document = session.load().clone();
        let mut authority = WorkerAuthority::Durable(Box::new(session));
        let mut coordinator = match document {
            JournalLoad::Known(ref document) if recovering => {
                Coordinator::recover(catalog, document).unwrap()
            }
            JournalLoad::Clean if !recovering => Coordinator::new(catalog),
            _ => panic!("unexpected native fixture journal bootstrap"),
        };
        if recovering {
            let JournalLoad::Known(document) = document else {
                unreachable!();
            };
            let plan = RecoveryPlan::resolve(&document, &mut ExpiredFixtureActor);
            assert!(!plan.blocked());
            assert_eq!(plan.release.len(), 1);
            let result = coordinator.apply_recovery(
                &plan,
                &mut RecoveryControl {
                    authority: &mut authority,
                },
            );
            assert_eq!(result.outcome, Outcome::Released);
            assert!(result.error.is_none());
            assert!(result.snapshot.owners.is_empty() && result.snapshot.resources.is_empty());
            worker_pause.pause();
        } else {
            use crate::app::shared::ids::{GroupId, RuleId};
            // Explicit synthetic fixture provenance, never a native boot/game
            // attestation. Parent Child::wait controls this fixture actor.
            let session = SessionIdentity {
                user: UserIdentity {
                    machine: machine(),
                    sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
                },
                session_id: 1,
                logon: LogonId::new(1).unwrap(),
                boot: BootId::new([3; 16]).unwrap(),
            };
            let process = ProcessIdentity::new(
                session.clone(),
                std::process::id(),
                1,
                ImagePath::new("C:\\Fixture\\owned-child.exe").unwrap(),
            )
            .unwrap();
            let binding = ActivationBinding::Game(
                GameActivation::new(
                    ConfiguredIntent {
                        profile: ProfileId::new(1).unwrap(),
                        generation: ConfigGeneration::new(1).unwrap(),
                    },
                    RuleActivation::new(
                        GroupId("fixture-group".into()),
                        RuleId("fixture-rule".into()),
                    )
                    .unwrap(),
                    ActivationEligibility::Running,
                    session,
                    vec![process],
                )
                .unwrap(),
            );
            ready.send((config, binding)).unwrap();
        }
        (coordinator, authority)
    })
    .unwrap();
    if recovering {
        boundary.recv_timeout(PROTOCOL_TIMEOUT).unwrap();
        announce(RECOVERED);
    } else {
        let (config, binding) = bootstrap.recv_timeout(PROTOCOL_TIMEOUT).unwrap();
        let ticket = runner
            .client()
            .submit(
                Command::Acquire {
                    binding,
                    owner: OwnerId::new(1).unwrap(),
                    config,
                    target: TargetIdentity::Machine(machine()),
                    mode: ActivationMode::Session,
                },
                1,
            )
            .unwrap();
        boundary.recv_timeout(PROTOCOL_TIMEOUT).unwrap();
        assert!(ticket.result().is_none());
        announce(PAUSED);
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        announce(TIMED_OUT);
    }
    // Keep the caller and exact blocked worker alive until the parent kills
    // this owned child. No timeout is treated as permission for fixture cleanup.
    let mut waiting = pause.waiting.lock().unwrap();
    loop {
        waiting = pause.wake.wait(waiting).unwrap();
    }
}
