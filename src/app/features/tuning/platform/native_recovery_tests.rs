//! Nonshipping forced-process interruption with real isolated metadata files.
//! The resource/game binding and resolver are fixtures, never native admission.
use super::{
    catalog::{Catalog, Module},
    codec::{ModuleEnvelope, VersionedCodec},
    configured_intent::{
        ConfiguredDecision, ConfiguredEntry, ConfiguredIntentSource, ConfiguredSnapshot,
    },
    contract::*,
    coordinator::{
        Command, CommandResult, Coordinator, CoordinatorSnapshot, ExecutionControl, GateError,
        Outcome, WriteIntent,
    },
    journal::{
        DurableSession, JournalDocument, JournalIoError, JournalIoKind, JournalLoad, JournalOwner,
        JournalPhase, JournalSource, JournalStamp, JournalStore, StoredJournal, MAX_JOURNAL_BYTES,
    },
    recovery::{OwnerAttachment, RecoveryPlan, RecoveryResolver, ResourceLifetime},
    runner::{Runner, RunnerLimits, ShutdownResult},
    writer::{bootstrap_configured, ScopeEvidence, WorkerAuthority},
};
use crate::app::features::tuning::testing::scalar::{
    ScalarConfig, ScalarModule, ScalarSnapshot, ScalarStep,
};
use os_api::tuning::{
    TuningNativeError, TuningProcessIdentity, TuningProcessLifetime, TuningProcessLifetimeGuard,
    TuningStore, TuningStoreAuthority, TuningTemporaryDirectory,
};
use std::{
    cell::RefCell,
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[path = "manual_recovery_lab_tests.rs"]
mod manual_lab;
#[test]
#[ignore = "explicit canonical manual recovery lab; never selected by ordinary tests"]
fn host_canonical_manual_recovery_lab() {
    manual_lab::run_host();
}

const CHILD_TEST: &str =
    "app::features::tuning::platform::native_recovery_tests::isolated_native_child";
const ROOT_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_ROOT";
const TARGET_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_TARGET";
const ROLE_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_ROLE";
const ACTOR_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_ACTOR";
const CONFIGURED_BINDING_ENV: &str = "CPU_AFFINITY_NATIVE_FIXTURE_CONFIGURED_BINDING";
const MAX_CONFIGURED_BINDING_BYTES: usize = 8192;
const PAUSED: &str = "NATIVE_FIXTURE_PAUSED";
const TIMED_OUT: &str = "NATIVE_FIXTURE_TIMED_OUT";
const RECOVERED: &str = "NATIVE_FIXTURE_RECOVERED";
const ACTOR_READY: &str = "NATIVE_FIXTURE_ACTOR_READY";
const REATTACHED: &str = "NATIVE_FIXTURE_REATTACHED";
const UNKNOWN: &str = "NATIVE_FIXTURE_UNKNOWN";
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
fn fixture_binding(process: ProcessIdentity) -> ActivationBinding {
    use crate::app::shared::ids::{GroupId, RuleId};
    ActivationBinding::Game(
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
            process.session.clone(),
            vec![process],
        )
        .unwrap(),
    )
}
fn fixture_process(pid: u32) -> ProcessIdentity {
    ProcessIdentity::new(
        SessionIdentity {
            user: UserIdentity {
                machine: machine(),
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(1).unwrap(),
            boot: BootId::new([3; 16]).unwrap(),
        },
        pid,
        1,
        ImagePath::new("C:\\Fixture\\owned-child.exe").unwrap(),
    )
    .unwrap()
}
struct FixtureConfigured(ConfiguredSnapshot);
impl ConfiguredIntentSource for FixtureConfigured {
    fn snapshot(&self) -> ConfiguredSnapshot {
        self.0.clone()
    }
}
fn configured_fixture(
    config: ModuleEnvelope,
    binding: ActivationBinding,
) -> Arc<dyn ConfiguredIntentSource> {
    Arc::new(FixtureConfigured(
        ConfiguredSnapshot::complete(
            1,
            vec![ConfiguredEntry {
                owner: OwnerId::new(1).unwrap(),
                binding,
                config,
                target: TargetIdentity::Machine(machine()),
                mode: ActivationMode::Session,
                decision: ConfiguredDecision::Enabled,
            }],
        )
        .unwrap(),
    ))
}
fn expected_binding() -> ActivationBinding {
    let input = std::env::var(CONFIGURED_BINDING_ENV)
        .expect("independent fixture configured input required");
    assert!(
        input.len() <= MAX_CONFIGURED_BINDING_BYTES,
        "bounded private fixture configured input"
    );
    serde_json::from_str(&input).expect("valid fixture binding input")
}
fn fixture_native_process(actual: &TuningProcessIdentity) -> ProcessIdentity {
    // Only PID/creation/image/token are native observations. Machine and boot
    // explicitly remain the isolated fixture domain, never native game proof.
    ProcessIdentity::new(
        SessionIdentity {
            user: UserIdentity {
                machine: machine(),
                sid: UserSid::new(actual.token().sid()).unwrap(),
            },
            session_id: actual.token().session_id(),
            logon: LogonId::new(actual.token().authentication_id()).unwrap(),
            boot: BootId::new([3; 16]).unwrap(),
        },
        actual.pid(),
        actual.creation_time(),
        ImagePath::new(actual.image_path()).unwrap(),
    )
    .unwrap()
}
fn native_fixture_attachment(
    owner: &JournalOwner,
    recorded: &ProcessIdentity,
    evidence: Result<TuningProcessLifetime, TuningNativeError>,
) -> OwnerAttachment {
    // The complete logical owner is a fixture. The only native producer input
    // is a result from the worker's retained, initially authenticated object.
    if owner.mode != ActivationMode::Session
        || recorded.session.user.machine != machine()
        || recorded.session.boot != BootId::new([3; 16]).unwrap()
        || owner.binding != fixture_binding(recorded.clone())
    {
        return OwnerAttachment::Unknown;
    }
    let ActivationBinding::Game(binding) = &owner.binding else {
        return OwnerAttachment::Unknown;
    };
    match evidence {
        Ok(TuningProcessLifetime::VerifiedLive) => OwnerAttachment::VerifiedLive {
            owner: owner.owner,
            binding: binding.clone(),
            live_processes: vec![recorded.clone()],
        },
        Ok(TuningProcessLifetime::PositivelyExited) => OwnerAttachment::PositivelyExpired {
            owner: owner.owner,
            binding: binding.clone(),
        },
        Err(_) => OwnerAttachment::Unknown,
    }
}
#[test]
fn native_actor_resolver_requires_exact_binding_and_retained_lifetime_evidence() {
    let recorded = ProcessIdentity::new(
        SessionIdentity {
            user: UserIdentity {
                machine: machine(),
                sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(1).unwrap(),
            boot: BootId::new([3; 16]).unwrap(),
        },
        42,
        91,
        ImagePath::new("C:\\Fixture\\shape.exe").unwrap(),
    )
    .unwrap();
    let owner = JournalOwner {
        owner: OwnerId::new(1).unwrap(),
        binding: fixture_binding(recorded.clone()),
        source: super::journal::SourceRef(0),
        mode: ActivationMode::Session,
        resources: Vec::new(),
        status: ModuleStatus::RecoveryRequired,
    };
    assert!(
        matches!(native_fixture_attachment(&owner,&recorded,Ok(TuningProcessLifetime::VerifiedLive)),
        OwnerAttachment::VerifiedLive {owner:candidate,live_processes,..} if candidate==owner.owner && live_processes==[recorded.clone()])
    );
    assert!(
        matches!(native_fixture_attachment(&owner,&recorded,Ok(TuningProcessLifetime::PositivelyExited)),
        OwnerAttachment::PositivelyExpired {owner:candidate,..} if candidate==owner.owner)
    );
    assert!(matches!(
        native_fixture_attachment(
            &owner,
            &recorded,
            Err(TuningNativeError::Unsupported("failed verify/open"))
        ),
        OwnerAttachment::Unknown
    ));
    let alias = ProcessIdentity::new(
        recorded.session.clone(),
        recorded.pid(),
        recorded.creation_token(),
        ImagePath::new("C:\\Fixture\\different.exe").unwrap(),
    )
    .unwrap();
    assert!(matches!(
        native_fixture_attachment(&owner, &alias, Ok(TuningProcessLifetime::VerifiedLive)),
        OwnerAttachment::Unknown
    ));
    assert!(matches!(
        native_fixture_attachment(&owner, &alias, Ok(TuningProcessLifetime::PositivelyExited)),
        OwnerAttachment::Unknown
    ));
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
    read_record(root).1
}
fn read_record(root: &Path) -> (Vec<u8>, JournalDocument) {
    let mut store = TuningStore::open_test_directory(root, MAX_JOURNAL_BYTES).unwrap();
    let bytes = store
        .read()
        .unwrap()
        .expect("interrupted durable record must survive");
    let JournalLoad::Known(document) = JournalDocument::load(bytes.clone(), &machine()) else {
        panic!("known fixture journal expected");
    };
    (bytes, document)
}

struct ProbeChild {
    child: Child,
    messages: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}
impl ProbeChild {
    fn spawn(root: &Path, target: &Path, role: &str) -> Self {
        Self::spawn_actor(root, target, role, None)
    }
    fn spawn_actor(root: &Path, target: &Path, role: &str, actor: Option<u32>) -> Self {
        Self::spawn_input(root, target, role, actor, None)
    }
    fn spawn_configured(
        root: &Path,
        target: &Path,
        role: &str,
        binding: &ActivationBinding,
    ) -> Self {
        Self::spawn_input(root, target, role, None, Some(binding))
    }
    fn spawn_input(
        root: &Path,
        target: &Path,
        role: &str,
        actor: Option<u32>,
        configured: Option<&ActivationBinding>,
    ) -> Self {
        let mut command = ProcessCommand::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(ROOT_ENV, root)
            .env(TARGET_ENV, target)
            .env(ROLE_ENV, role)
            .env_remove(CONFIGURED_BINDING_ENV)
            .env_remove(ACTOR_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(0x0800_0000);
        if let Some(actor) = actor {
            command.env(ACTOR_ENV, actor.to_string());
        }
        if let Some(binding) = configured {
            let input = serde_json::to_string(binding).unwrap();
            assert!(
                input.len() <= MAX_CONFIGURED_BINDING_BYTES,
                "bounded private fixture configured input"
            );
            command.env(CONFIGURED_BINDING_ENV, input);
        }
        let mut child = command.spawn().unwrap();
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
    let configured = fixture_binding(fixture_process(writer.child.id()));
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
    let mut recovery = ProbeChild::spawn_configured(
        journal_root.path(),
        &target,
        "recover-fixture-expired",
        &configured,
    );
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
fn retained_actor_interruption(ended_before_guard: bool) {
    let journal_root = TuningTemporaryDirectory::new().unwrap();
    let target_root = TuningTemporaryDirectory::new().unwrap();
    let target = target_root.path().join("scalar-native-target.bin");
    std::fs::write(&target, 7i32.to_le_bytes()).unwrap();
    let mut actor = ProbeChild::spawn(journal_root.path(), &target, "actor");
    actor.wait_message(ACTOR_READY);
    let expected = os_api::OS::tuning_process_identity(actor.child.id()).unwrap();
    let mut writer = ProbeChild::spawn_actor(
        journal_root.path(),
        &target,
        "progress-native-actor",
        Some(actor.child.id()),
    );
    writer.wait_message(PAUSED);
    writer.wait_message(TIMED_OUT);
    assert!(matches!(
        TuningStore::open_test_directory(journal_root.path(), MAX_JOURNAL_BYTES),
        Err(TuningNativeError::Busy)
    ));
    writer.terminate();
    assert!(
        actor.child.try_wait().unwrap().is_none(),
        "separate owned actor survived writer"
    );
    let (original_bytes, original) = read_record(journal_root.path());
    let ActivationBinding::Game(binding) = &original.owners[0].binding else {
        panic!("fixture owner binding");
    };
    assert_eq!(binding.processes().len(), 1);
    let recorded = &binding.processes()[0];
    // Independent native oracle comparisons, without dumping actual values.
    assert!(recorded.pid() == expected.pid(), "recorded native PID");
    assert!(
        recorded.creation_token() == expected.creation_time(),
        "recorded full creation time"
    );
    assert!(
        recorded.image_path.as_str() == expected.image_path(),
        "recorded exact native image"
    );
    assert!(
        recorded.session.user.sid.as_str() == expected.token().sid(),
        "recorded native token SID"
    );
    assert!(
        recorded.session.session_id == expected.token().session_id(),
        "recorded native session"
    );
    assert!(
        recorded.session.logon.get() == expected.token().authentication_id(),
        "recorded full native authentication LUID"
    );
    assert!(
        recorded.session.user.machine == machine()
            && recorded.session.boot == BootId::new([3; 16]).unwrap(),
        "explicit fixture machine/boot domain"
    );
    assert!(matches!(
        original.phase,
        JournalPhase::VerifiedWrite { step: 0, .. }
    ));
    assert_eq!(original.resources[0].baseline.bytes(), 7i32.to_le_bytes());
    assert_eq!(original.resources[0].confirmed.bytes(), 12i32.to_le_bytes());
    assert_eq!(read_target(&target), 12);
    if ended_before_guard {
        actor.terminate();
    }
    let role = if ended_before_guard {
        "recover-native-unknown"
    } else {
        "recover-native-live"
    };
    let configured = fixture_binding(fixture_native_process(&expected));
    let mut recovery =
        ProbeChild::spawn_configured(journal_root.path(), &target, role, &configured);
    recovery.wait_message(if ended_before_guard {
        UNKNOWN
    } else {
        REATTACHED
    });
    recovery.wait_message(TIMED_OUT);
    assert_eq!(read_target(&target), 12, "no premature restoration");
    assert!(matches!(
        TuningStore::open_test_directory(journal_root.path(), MAX_JOURNAL_BYTES),
        Err(TuningNativeError::Busy)
    ));
    if !ended_before_guard {
        assert!(actor.child.try_wait().unwrap().is_none());
        actor.terminate(); // Only the worker's SAME authenticated guard can attest expiry.
        recovery.wait_message(RECOVERED);
        assert_eq!(read_target(&target), 7);
        assert!(matches!(
            TuningStore::open_test_directory(journal_root.path(), MAX_JOURNAL_BYTES),
            Err(TuningNativeError::Busy)
        ));
    }
    recovery.terminate();
    let (final_bytes, final_document) = read_record(journal_root.path());
    if ended_before_guard {
        assert!(
            final_bytes == original_bytes && final_document == original,
            "Unknown preserves exact journal and original baseline"
        );
        assert_eq!(read_target(&target), 12);
    } else {
        assert_eq!(final_document.stamp, JournalStamp::Recovery);
        assert!(final_document.owners.is_empty() && final_document.resources.is_empty());
        assert!(final_document.revision > original.revision);
    }
    drop(recovery);
    drop(writer);
    drop(actor);
    target_root.cleanup().unwrap();
    journal_root.cleanup().unwrap();
}
#[test]
fn retained_native_actor_survives_writer_and_same_guard_releases_fixture_after_exit() {
    retained_actor_interruption(false);
}
#[test]
fn native_actor_ended_before_guard_keeps_unknown_journal_and_target_unchanged() {
    retained_actor_interruption(true);
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
    target: PhysicalTarget,
    pause_after_write: Option<Arc<PauseGate>>,
    rehydrated: bool,
    writes: Option<Arc<AtomicUsize>>,
}
enum PhysicalTarget {
    Fixture(PathBuf),
    Native(Arc<os_api::tuning::TuningLabScalarIo>),
}
impl From<PathBuf> for PhysicalTarget {
    fn from(path: PathBuf) -> Self {
        Self::Fixture(path)
    }
}
impl PhysicalTarget {
    fn read(&self) -> Result<i32, ModuleError> {
        match self {
            Self::Fixture(path) => Ok(read_target(path)),
            Self::Native(io) => io
                .read()
                .map_err(|_| ModuleError::new(ErrorKind::Read, "owned scalar read")),
        }
    }
    fn write(&self, value: i32) -> Result<(), ModuleError> {
        match self {
            Self::Fixture(path) => {
                write_target(path, value);
                Ok(())
            }
            Self::Native(io) => io
                .write(value)
                .map_err(|_| ModuleError::new(ErrorKind::Write, "owned scalar write")),
        }
    }
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
        self.scalar.external_change(self.target.read()?);
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
        if self.rehydrated {
            return Err(ModuleError::new(
                ErrorKind::InvalidPlan,
                "recovery baseline recapture refused",
            ));
        }
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
        self.scalar.external_change(self.target.read()?);
        let effect = self.scalar.execute_step(target, step)?;
        self.target.write(self.scalar.value())?;
        if let Some(writes) = &self.writes {
            writes.fetch_add(1, Ordering::SeqCst);
        }
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

/// Test-only rule/resource ownership; native authority covers only the exact
/// recorded process object's lifetime. It cannot prove a game, boot or logon.
struct NativeFixtureActor {
    recorded: ProcessIdentity,
    guard: Result<TuningProcessLifetimeGuard, TuningNativeError>,
}
impl NativeFixtureActor {
    fn open(recorded: ProcessIdentity) -> Self {
        let guard = (|| {
            let actual = os_api::OS::tuning_process_identity(recorded.pid())?;
            if fixture_native_process(&actual) != recorded {
                return Err(TuningNativeError::Unsupported("fixture process mismatch"));
            }
            os_api::OS::tuning_open_process_lifetime(&actual)
        })();
        Self { recorded, guard }
    }
    fn poll(&mut self) -> Result<TuningProcessLifetime, TuningNativeError> {
        match &mut self.guard {
            Ok(guard) => guard.poll(),
            Err(error) => Err(error.clone()),
        }
    }
}
impl RecoveryResolver for NativeFixtureActor {
    fn owner(&mut self, owner: &JournalOwner, _: &JournalSource) -> OwnerAttachment {
        let evidence = self.poll();
        native_fixture_attachment(owner, &self.recorded, evidence)
    }
    fn resource(&mut self, _: &ResourceKey) -> ResourceLifetime {
        // The isolated machine scalar remains present independently of actor exit.
        ResourceLifetime::VerifiedLive
    }
}
struct SharedNativeFixtureActor(Rc<RefCell<NativeFixtureActor>>);
impl RecoveryResolver for SharedNativeFixtureActor {
    fn owner(&mut self, owner: &JournalOwner, source: &JournalSource) -> OwnerAttachment {
        self.0.borrow_mut().owner(owner, source)
    }
    fn resource(&mut self, key: &ResourceKey) -> ResourceLifetime {
        self.0.borrow_mut().resource(key)
    }
}
fn authority_document(authority: &WorkerAuthority) -> JournalDocument {
    authority
        .retained_document()
        .expect("fixture retained durable document")
        .clone()
}
fn native_actor_recovery_child(root: PathBuf, target: PathBuf, unknown: bool) {
    let (reached, boundary) = mpsc::sync_channel(1);
    let pause = Arc::new(PauseGate {
        reached,
        waiting: Mutex::new(()),
        wake: Condvar::new(),
    });
    let worker_pause = pause.clone();
    let mut runner = Runner::start_test_writer(RunnerLimits::default(), move || {
        let native = TuningStore::open_test_directory(&root, MAX_JOURNAL_BYTES).unwrap();
        assert_eq!(native.authority(), TuningStoreAuthority::IsolatedTest);
        let mut native = native;
        let bytes = native.read().unwrap().expect("recorded fixture journal");
        let JournalLoad::Known(original) = JournalDocument::load(bytes, &machine()) else {
            panic!("recorded fixture journal");
        };
        let writes = Arc::new(AtomicUsize::new(0));
        let module = FileScalarModule {
            scalar: ScalarModule::new(read_target(&target)),
            target: target.clone().into(),
            pause_after_write: None,
            rehydrated: true,
            writes: Some(writes.clone()),
        };
        let config = module
            .scalar
            .config_envelope(&ScalarConfig { value: 12 })
            .unwrap();
        // This input was captured independently by the parent before the
        // interruption. The journal cannot authorize its own owner.
        let binding = expected_binding();
        let ActivationBinding::Game(game) = &binding else {
            panic!("independent fixture binding");
        };
        assert_eq!(game.processes().len(), 1);
        let resolver = Rc::new(RefCell::new(NativeFixtureActor::open(
            game.processes()[0].clone(),
        )));
        if unknown {
            assert!(
                resolver.borrow().guard.is_err(),
                "actual verify/open failure is required"
            );
        } else {
            assert!(
                resolver.borrow().guard.is_ok(),
                "guard acquired while separate actor live"
            );
        }
        let mut catalog = Catalog::default();
        catalog.register(module).unwrap();
        let store: Box<dyn JournalStore> = Box::new(IsolatedStore {
            native,
            pause: None,
        });
        let (mut coordinator, mut authority) = bootstrap_configured(
            catalog,
            machine(),
            store,
            configured_fixture(config, binding),
            ScopeEvidence::fixture_lifetime(
                || Ok(machine()),
                Box::new(SharedNativeFixtureActor(resolver.clone())),
            ),
        );
        let plan =
            RecoveryPlan::resolve(&original, &mut SharedNativeFixtureActor(resolver.clone()));
        assert!(
            plan.retire.is_empty(),
            "actor lifetime cannot retire projected fixture resource"
        );
        if unknown {
            assert!(plan.blocked() && plan.release.is_empty() && plan.reattach.is_empty());
            // The common bootstrap already resolves and applies the recovery
            // decision; no permissive manual second bootstrap is available.
            assert!(
                matches!(authority.status(), super::writer::WriterStatus::Blocked(_)),
                "failed guard acquisition must close the common durable worker admission"
            );
            assert!(
                authority_document(&authority) == original,
                "Unknown retains exact durable input"
            );
            assert_eq!(writes.load(Ordering::SeqCst), 0);
            assert_eq!(read_target(&target), 12);
            worker_pause.pause();
        } else {
            assert!(!plan.blocked() && plan.release.is_empty());
            assert_eq!(plan.reattach, [OwnerId::new(1).unwrap()]);
            assert!(!matches!(
                authority.status(),
                super::writer::WriterStatus::Blocked(_)
            ));
            let snapshot = coordinator.snapshot();
            assert_eq!(snapshot.resources[0].baseline.bytes(), 7i32.to_le_bytes());
            assert_eq!(snapshot.resources[0].confirmed.bytes(), 12i32.to_le_bytes());
            let attached = authority_document(&authority);
            assert!(
                attached.sources == original.sources
                    && attached.owners[0].binding == original.owners[0].binding,
                "reattachment preserves recorded actor and first baseline sources"
            );
            assert_eq!(
                writes.load(Ordering::SeqCst),
                0,
                "zero premature target write"
            );
            assert_eq!(read_target(&target), 12);
            worker_pause.reached.send(()).unwrap();
            let deadline = Instant::now() + PROTOCOL_TIMEOUT;
            loop {
                match resolver.borrow_mut().poll() {
                    Ok(TuningProcessLifetime::VerifiedLive) => (),
                    Ok(TuningProcessLifetime::PositivelyExited) => break,
                    Err(_) => {
                        // Exit can race the live tuple query. Unknown permits no
                        // cleanup; keep this SAME guard/lease until its native
                        // handle poll positively proves exit or deadline fails.
                        assert_eq!(writes.load(Ordering::SeqCst), 0);
                        assert_eq!(read_target(&target), 12);
                    }
                }
                assert!(Instant::now() < deadline, "owned actor expiry deadline");
                thread::yield_now();
            }
            let expired =
                RecoveryPlan::resolve(&attached, &mut SharedNativeFixtureActor(resolver.clone()));
            assert!(!expired.blocked() && expired.reattach.is_empty() && expired.retire.is_empty());
            assert_eq!(expired.release, [OwnerId::new(1).unwrap()]);
            let result =
                apply_fixture_recovery(&mut coordinator, &mut authority, &expired).unwrap();
            assert_eq!(result.outcome, Outcome::Released);
            assert!(result.error.is_none());
            assert_eq!(
                writes.load(Ordering::SeqCst),
                1,
                "one verified physical restore"
            );
            assert_eq!(read_target(&target), 7);
            let released = authority_document(&authority);
            assert!(
                released.owners.is_empty()
                    && released.resources.is_empty()
                    && released.revision > attached.revision
            );
            announce(RECOVERED);
            worker_pause.pause();
        }
        (coordinator, authority)
    })
    .unwrap();
    boundary.recv_timeout(PROTOCOL_TIMEOUT).unwrap();
    announce(if unknown { UNKNOWN } else { REATTACHED });
    assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
    announce(TIMED_OUT);
    let mut waiting = pause.waiting.lock().unwrap();
    loop {
        waiting = pause.wake.wait(waiting).unwrap();
    }
}
// This direct fixture callback starts a separate operation after bootstrap
// returns idle. It is also exercised without native I/O by the state regression.
pub(super) fn apply_fixture_recovery(
    coordinator: &mut Coordinator,
    authority: &mut WorkerAuthority,
    plan: &RecoveryPlan,
) -> Result<CommandResult, GateError> {
    authority.begin_shutdown(&coordinator.snapshot())?;
    let result = coordinator.apply_recovery(plan, &mut RecoveryControl { authority });
    authority.finish_operation();
    Ok(result)
}
impl ExecutionControl for RecoveryControl<'_> {
    fn cancellation_requested(&self) -> bool {
        false
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.authority.check_write(intent)?;
        self.authority.commit(
            JournalStamp::Recovery,
            JournalPhase::BeforeWrite {
                step: u32::try_from(intent.step).unwrap(),
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )
    }
    fn before_execute(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
        self.authority.after_intent()
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
    if role == "actor" {
        announce(ACTOR_READY);
        loop {
            thread::park();
        }
    }
    if matches!(
        role.as_str(),
        "recover-native-live" | "recover-native-unknown"
    ) {
        native_actor_recovery_child(root, target, role == "recover-native-unknown");
        return;
    }
    let recovering = role == "recover-fixture-expired";
    let native_actor = (role == "progress-native-actor")
        .then(|| std::env::var(ACTOR_ENV).unwrap().parse::<u32>().unwrap());
    let point = (!recovering).then(|| {
        if native_actor.is_some() {
            StopPoint::Progress
        } else {
            StopPoint::parse(&role)
        }
    });
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
            target: target.into(),
            pause_after_write: (point == Some(StopPoint::Write)).then(|| worker_pause.clone()),
            rehydrated: recovering,
            writes: None,
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
        let binding = if recovering {
            expected_binding()
        } else if let Some(pid) = native_actor {
            fixture_binding(fixture_native_process(
                &os_api::OS::tuning_process_identity(pid).unwrap(),
            ))
        } else {
            fixture_binding(fixture_process(std::process::id()))
        };
        let (coordinator, authority) = bootstrap_configured(
            catalog,
            machine(),
            store,
            configured_fixture(config.clone(), binding.clone()),
            ScopeEvidence::fixture_lifetime(|| Ok(machine()), Box::new(ExpiredFixtureActor)),
        );
        assert!(
            !matches!(authority.status(), super::writer::WriterStatus::Blocked(_)),
            "common fixture bootstrap must complete recovery"
        );
        if recovering {
            let snapshot = coordinator.snapshot();
            assert!(snapshot.owners.is_empty() && snapshot.resources.is_empty());
            assert!(authority_document(&authority).owners.is_empty());
            worker_pause.pause();
        } else {
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
