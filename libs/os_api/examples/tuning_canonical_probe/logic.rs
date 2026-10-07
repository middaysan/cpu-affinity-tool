use os_api::tuning::TuningNativeError;
use serde::Serialize;
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};

pub const OPEN_DEADLINE: Duration = Duration::from_secs(10);
pub const HOLD_DEADLINE: Duration = Duration::from_secs(30);
pub const MAX_REPORT_BYTES: usize = 1024;
pub const MAX_CONTROL_BYTES: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Preflight,
    LabOpen,
    LabHold,
    LabContend,
    LabContinuityObserve,
    LabContinuityInitialize,
    LabContinuityCompare,
}
pub fn parse_mode(args: &[String]) -> Result<Mode, ()> {
    match args {
        [] => Ok(Mode::Preflight),
        [value] => match value.as_str() {
            "preflight" => Ok(Mode::Preflight),
            "lab-open" => Ok(Mode::LabOpen),
            "lab-hold" => Ok(Mode::LabHold),
            "lab-contend" => Ok(Mode::LabContend),
            "lab-continuity-observe" => Ok(Mode::LabContinuityObserve),
            "lab-continuity-initialize" => Ok(Mode::LabContinuityInitialize),
            "lab-continuity-compare" => Ok(Mode::LabContinuityCompare),
            _ => Err(()),
        },
        _ => Err(()),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Elevation {
    Standard,
    Elevated,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    Untrusted,
    Low,
    Medium,
    High,
    System,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TokenContext {
    pub elevation: Elevation,
    pub integrity: Integrity,
}
impl TokenContext {
    pub const UNKNOWN: Self = Self {
        elevation: Elevation::Unknown,
        integrity: Integrity::Unknown,
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preflight,
    Open,
    Authority,
    Validate,
    Read,
    Continuity,
    Hold,
    Closed,
    Controller,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ReadOnlyStandard,
    SkippedElevated,
    UnknownToken,
    LabContextDenied,
    Opened,
    Ready,
    Busy,
    UnexpectedLeaseAcquired,
    NativeFailure,
    NonCanonical,
    Completed,
    ControlRejected,
    ControlTimedOut,
    Timeout,
    InvalidArguments,
    #[cfg(not(windows))]
    UnsupportedPlatform,
    WorkerPanicked,
    UnsupportedBuild,
    ContinuityStable,
    ContinuityUnknown,
    ContinuityChanged,
    ContinuityInitializationBlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityMode {
    ObserveExisting,
    InitializeWhenAbsent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityCheck {
    Stable,
    Unknown,
    Changed,
}

/// Comparison of private full observations; shape fixtures issue no authority.
pub fn continuity_check<T: Eq>(pair: [Option<T>; 2]) -> ContinuityCheck {
    match pair {
        [Some(first), Some(second)] if first == second => ContinuityCheck::Stable,
        [Some(_), Some(_)] => ContinuityCheck::Changed,
        _ => ContinuityCheck::Unknown,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    NotOpened,
    Canonical,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "bytes", rename_all = "snake_case")]
pub enum Presence {
    NotRead,
    Absent,
    PresentUninterpreted(u64),
    Oversized(u64),
    Unreadable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Busy,
    Unsupported,
    Unsafe,
    Poisoned,
    Capacity,
    Denied,
    Io,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreRefusal {
    OwnerUntrusted,
    DaclMissing,
    DaclUnprotected,
    DaclEmpty,
    MutationGrantUntrusted,
    AuthorityGrantMissing,
    IntegrityLabelInvalid,
    AclMissing,
    AceMalformed,
    AceSidMalformed,
    ObjectTopologyInvalid,
    ObjectChanged,
    NamespaceChanged,
    ArtifactsUnexpected,
    PathInvalid,
}
fn store_refusal(reason: &'static str) -> Option<StoreRefusal> {
    // Exact internal literals map to fixed predicates; arbitrary text is omitted.
    Some(match reason {
        "untrusted object owner" => StoreRefusal::OwnerUntrusted,
        "missing DACL" => StoreRefusal::DaclMissing,
        "unprotected managed DACL" => StoreRefusal::DaclUnprotected,
        "empty DACL" => StoreRefusal::DaclEmpty,
        "untrusted storage mutation grant" => StoreRefusal::MutationGrantUntrusted,
        "missing authority grant" => StoreRefusal::AuthorityGrantMissing,
        "missing high integrity no-write-up" => StoreRefusal::IntegrityLabelInvalid,
        "missing ACL" => StoreRefusal::AclMissing,
        "malformed ACE" => StoreRefusal::AceMalformed,
        "malformed ACE SID" => StoreRefusal::AceSidMalformed,
        "unexpected object type/reparse/hardlink" => StoreRefusal::ObjectTopologyInvalid,
        "held directory changed" | "held lease changed" => StoreRefusal::ObjectChanged,
        "directory namespace changed" | "lease namespace changed" => StoreRefusal::NamespaceChanged,
        "unknown managed artifacts" | "pending or unknown journal artifact" => {
            StoreRefusal::ArtifactsUnexpected
        }
        "absolute store required"
        | "normalized store components required"
        | "invalid native path" => StoreRefusal::PathInvalid,
        _ => return None,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityRefusal {
    MalformedCapsule,
    QueryBound,
    UnsafeSecurity,
    ContextUnproven,
    MissingComponent,
    InitializationRefused,
    ObservationUnknown,
    PublisherDomainUnproven,
    MachineSourceUnavailable,
    MachineBytesInvalid,
}
fn continuity_refusal(reason: &'static str) -> Option<ContinuityRefusal> {
    // Fixed refusal classes report no raw text or positive native proof.
    Some(match reason {
        "canonical continuity malformed capsule" => ContinuityRefusal::MalformedCapsule,
        "canonical continuity query bounds" => ContinuityRefusal::QueryBound,
        "canonical continuity unsafe security" => ContinuityRefusal::UnsafeSecurity,
        "canonical continuity context unproven" => ContinuityRefusal::ContextUnproven,
        "canonical continuity missing component" => ContinuityRefusal::MissingComponent,
        "canonical continuity initialization refused" => ContinuityRefusal::InitializationRefused,
        "canonical continuity observation unknown" => ContinuityRefusal::ObservationUnknown,
        "publisher domain is not proven unpackaged" => ContinuityRefusal::PublisherDomainUnproven,
        "system ID source is unavailable" => ContinuityRefusal::MachineSourceUnavailable,
        "system ID byte length" => ContinuityRefusal::MachineBytesInvalid,
        _ => return None,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub code: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_refusal: Option<StoreRefusal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuity_refusal: Option<ContinuityRefusal>,
}
impl From<TuningNativeError> for Failure {
    fn from(error: TuningNativeError) -> Self {
        let refusal = match &error {
            TuningNativeError::UnsafeStore(reason) => store_refusal(reason),
            _ => None,
        };
        let continuity_refusal = match &error {
            TuningNativeError::Unsupported(reason) => continuity_refusal(reason),
            _ => None,
        };
        let (kind, code) = match error {
            TuningNativeError::Busy => (FailureKind::Busy, None),
            TuningNativeError::Unsupported(_) => (FailureKind::Unsupported, None),
            TuningNativeError::UnsafeStore(_) => (FailureKind::Unsafe, None),
            TuningNativeError::Poisoned => (FailureKind::Poisoned, None),
            TuningNativeError::Capacity { .. } => (FailureKind::Capacity, None),
            TuningNativeError::Denied { code, .. } => (FailureKind::Denied, Some(code)),
            TuningNativeError::Io { code, .. } => (FailureKind::Io, Some(code)),
        };
        Self {
            kind,
            code,
            store_refusal: refusal,
            continuity_refusal,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub schema: u32,
    pub command: Mode,
    pub stage: Stage,
    pub outcome: Outcome,
    pub token: TokenContext,
    pub store_attempted: bool,
    pub authority: Authority,
    pub journal: Presence,
    pub failure: Option<Failure>,
    pub worker_terminal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuity: Option<ContinuityCheck>,
}
impl Report {
    pub fn new(command: Mode, stage: Stage, outcome: Outcome) -> Self {
        Self {
            schema: 1,
            command,
            stage,
            outcome,
            token: TokenContext::UNKNOWN,
            store_attempted: false,
            authority: Authority::NotOpened,
            journal: Presence::NotRead,
            failure: None,
            worker_terminal: false,
            continuity: None,
        }
    }
    pub fn successful(&self) -> bool {
        if matches!(
            self.command,
            Mode::LabContinuityObserve | Mode::LabContinuityInitialize
        ) {
            return self.outcome == Outcome::ContinuityStable
                && self.continuity == Some(ContinuityCheck::Stable);
        }
        matches!(
            self.outcome,
            Outcome::ReadOnlyStandard
                | Outcome::SkippedElevated
                | Outcome::Opened
                | Outcome::Busy
                | Outcome::Completed
        )
    }
    pub fn line(&self) -> Result<String, ()> {
        let encoded = serde_json::to_string(self).map_err(|_| ())?;
        if encoded.len() > MAX_REPORT_BYTES {
            return Err(());
        }
        Ok(encoded)
    }
}

pub fn terminal_exit_code(
    report: &Report,
    joined: bool,
    controller_timed_out: bool,
    complete_output: bool,
) -> i32 {
    if joined
        && report.worker_terminal
        && !controller_timed_out
        && complete_output
        && report.successful()
    {
        0
    } else {
        2
    }
}

/// No journal publication, repair or deletion method is exposed.
pub trait ProbeStore {
    fn canonical(&self) -> bool;
    fn validate(&mut self) -> Result<(), Failure>;
    fn presence(&mut self) -> Result<Presence, Failure>;
    fn continuity(&mut self, mode: ContinuityMode) -> Result<ContinuityCheck, Failure>;
}
pub trait Backend {
    type Store: ProbeStore;
    fn continuity_supported_build(&self) -> bool;
    fn token_context(&mut self) -> Result<TokenContext, Failure>;
    fn open_canonical(&mut self) -> Result<Self::Store, Failure>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Release,
    Invalid,
}
pub fn read_control(reader: &mut impl std::io::Read) -> Control {
    let mut frame = [0; MAX_CONTROL_BYTES];
    for index in 0..frame.len() {
        match reader.read(&mut frame[index..index + 1]) {
            Ok(1) => {
                if frame[index] == b'\n' {
                    return if matches!(&frame[..=index], b"RELEASE\n" | b"RELEASE\r\n") {
                        Control::Release
                    } else {
                        Control::Invalid
                    };
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                return Control::Invalid;
            }
            _ => return Control::Invalid,
        }
    }
    Control::Invalid
}
pub fn classify_integrity_sid(sid: &[u8]) -> Integrity {
    if sid.len() != 12 || sid[..8] != [1, 1, 0, 0, 0, 0, 0, 16] {
        return Integrity::Unknown;
    }
    match u32::from_le_bytes(sid[8..12].try_into().expect("fixed SID RID")) {
        0 => Integrity::Untrusted,
        0x1000 => Integrity::Low,
        0x2000 | 0x2100 => Integrity::Medium,
        0x3000 => Integrity::High,
        0x4000 => Integrity::System,
        _ => Integrity::Unknown,
    }
}
pub struct Worker {
    pub reports: Receiver<Report>,
    pub control: SyncSender<Control>,
    handle: JoinHandle<()>,
}
impl Worker {
    pub fn start<B: Backend + 'static>(
        mode: Mode,
        factory: impl FnOnce() -> B + Send + 'static,
    ) -> Self {
        Self::start_with_hold_deadline(mode, factory, HOLD_DEADLINE)
    }
    fn start_with_hold_deadline<B: Backend + 'static>(
        mode: Mode,
        factory: impl FnOnce() -> B + Send + 'static,
        hold_deadline: Duration,
    ) -> Self {
        let (sender, reports) = mpsc::sync_channel(4);
        let (control, receiver) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let mut backend = factory();
            let mut report = run_probe(mode, &mut backend, &receiver, &sender, hold_deadline);
            drop(backend);
            report.worker_terminal = true;
            let _ = sender.send(report);
        });
        Self {
            reports,
            control,
            handle,
        }
    }
    pub fn join(self) -> bool {
        self.handle.join().is_ok()
    }
}
fn run_probe<B: Backend>(
    mode: Mode,
    backend: &mut B,
    control: &Receiver<Control>,
    reports: &SyncSender<Report>,
    hold_deadline: Duration,
) -> Report {
    let mut report = Report::new(mode, Stage::Preflight, Outcome::UnknownToken);
    if mode == Mode::LabContinuityCompare {
        report.outcome = Outcome::InvalidArguments;
        return report;
    }
    let continuity_mode = match mode {
        Mode::LabContinuityObserve => Some(ContinuityMode::ObserveExisting),
        Mode::LabContinuityInitialize => Some(ContinuityMode::InitializeWhenAbsent),
        _ => None,
    };
    if continuity_mode.is_some() && !backend.continuity_supported_build() {
        report.outcome = Outcome::UnsupportedBuild;
        return report;
    }
    report.token = match backend.token_context() {
        Ok(token) => token,
        Err(failure) => {
            report.failure = Some(failure);
            return report;
        }
    };
    if report.token.elevation == Elevation::Unknown || report.token.integrity == Integrity::Unknown
    {
        return report;
    }
    // Default/preflight never attempts canonical initialization, for any token.
    if mode == Mode::Preflight {
        report.outcome = match report.token.elevation {
            Elevation::Standard => Outcome::ReadOnlyStandard,
            Elevation::Elevated => Outcome::SkippedElevated,
            Elevation::Unknown => Outcome::UnknownToken,
        };
        return report;
    }
    if report.token.elevation != Elevation::Elevated
        || !matches!(report.token.integrity, Integrity::High | Integrity::System)
    {
        report.outcome = Outcome::LabContextDenied;
        return report;
    }
    report.stage = Stage::Open;
    report.store_attempted = true;
    let mut store = match backend.open_canonical() {
        Ok(store) => store,
        Err(failure) => {
            report.outcome = if mode == Mode::LabContend && failure.kind == FailureKind::Busy {
                Outcome::Busy
            } else {
                Outcome::NativeFailure
            };
            report.failure = Some(failure);
            return report;
        }
    };
    report.stage = Stage::Authority;
    if !store.canonical() {
        report.authority = Authority::Rejected;
        report.outcome = Outcome::NonCanonical;
        drop(store);
        return report;
    }
    report.authority = Authority::Canonical;
    if mode == Mode::LabContend {
        report.outcome = Outcome::UnexpectedLeaseAcquired;
        drop(store);
        return report;
    }
    report.stage = Stage::Validate;
    if let Err(failure) = store.validate() {
        report.outcome = Outcome::NativeFailure;
        report.failure = Some(failure);
        drop(store);
        return report;
    }
    report.stage = Stage::Read;
    report.journal = match store.presence() {
        Ok(presence) => presence,
        Err(failure) => {
            report.journal = Presence::Unreadable;
            report.outcome = Outcome::NativeFailure;
            report.failure = Some(failure);
            drop(store);
            return report;
        }
    };
    if !matches!(
        report.journal,
        Presence::Absent | Presence::PresentUninterpreted(_)
    ) {
        report.outcome = Outcome::NativeFailure;
        report.failure = Some(Failure {
            kind: FailureKind::Capacity,
            code: None,
            store_refusal: None,
            continuity_refusal: None,
        });
        drop(store);
        return report;
    }
    if let Some(intent) = continuity_mode {
        if intent == ContinuityMode::InitializeWhenAbsent && report.journal != Presence::Absent {
            report.outcome = Outcome::ContinuityInitializationBlocked;
            drop(store);
            return report;
        }
        report.stage = Stage::Continuity;
        match store.continuity(intent) {
            Ok(check) => {
                report.continuity = Some(check);
                report.outcome = match check {
                    ContinuityCheck::Stable => Outcome::ContinuityStable,
                    ContinuityCheck::Unknown => Outcome::ContinuityUnknown,
                    ContinuityCheck::Changed => Outcome::ContinuityChanged,
                };
            }
            Err(failure) => {
                report.outcome = Outcome::NativeFailure;
                report.failure = Some(failure);
            }
        }
        drop(store);
        return report;
    }
    if mode == Mode::LabHold {
        report.stage = Stage::Hold;
        report.outcome = Outcome::Ready;
        if reports.send(report).is_err() {
            report.outcome = Outcome::ControlRejected;
        } else {
            report.outcome = match control.recv_timeout(hold_deadline) {
                Ok(Control::Release) => Outcome::Completed,
                Ok(Control::Invalid) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    Outcome::ControlRejected
                }
                Err(mpsc::RecvTimeoutError::Timeout) => Outcome::ControlTimedOut,
            };
        }
        report.stage = Stage::Closed;
    } else {
        report.outcome = Outcome::Opened;
    }
    // The enclosing worker publishes terminal only after this native authority
    // and the backend have been dropped on that same worker.
    drop(store);
    report
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// Private child-pipe evidence only. Neither decoding nor fixtures issue native authority.
pub const SAMPLE_HANDSHAKE: &[u8] = b"CMP-SAMPLE-1\n";
pub const PRIVATE_SAMPLE_ARGUMENT: &str = "--private-continuity-sample";
pub const MAX_SAMPLE_BYTES: usize = 4352;
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateSample {
    pub machine_source: u8,
    pub machine_domain: u8,
    pub machine_bytes: Vec<u8>,
    pub producer: String,
    pub version: u32,
    pub domain: String,
    pub view: String,
    pub nonce: [u8; 32],
}
impl std::fmt::Debug for PrivateSample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateSample { evidence: redacted }")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonResult {
    Equal,
    Different,
    Failed,
    TimedOut,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ComparisonReport {
    pub schema: u32,
    pub command: Mode,
    pub result: ComparisonResult,
    pub machine_equal: Option<bool>,
    pub continuity_equal: Option<bool>,
    pub worker_terminal: bool,
}
impl ComparisonReport {
    pub fn failed(result: ComparisonResult) -> Self {
        Self {
            schema: 1,
            command: Mode::LabContinuityCompare,
            result,
            machine_equal: None,
            continuity_equal: None,
            worker_terminal: false,
        }
    }
    pub fn exit_code(&self) -> i32 {
        if self.result == ComparisonResult::Equal
            && self.machine_equal == Some(true)
            && self.continuity_equal == Some(true)
            && self.worker_terminal
        {
            0
        } else {
            2
        }
    }
    pub fn line(&self) -> Result<String, ()> {
        let line = serde_json::to_string(self).map_err(|_| ())?;
        if line.len() <= MAX_REPORT_BYTES {
            Ok(line)
        } else {
            Err(())
        }
    }
}
pub fn compare_samples(first: &PrivateSample, second: &PrivateSample) -> ComparisonReport {
    let machine_equal = first.machine_source == second.machine_source
        && first.machine_domain == second.machine_domain
        && first.machine_bytes == second.machine_bytes;
    let continuity_equal = first.producer == second.producer
        && first.version == second.version
        && first.domain == second.domain
        && first.view == second.view
        && first.nonce == second.nonce;
    ComparisonReport {
        schema: 1,
        command: Mode::LabContinuityCompare,
        result: if machine_equal && continuity_equal {
            ComparisonResult::Equal
        } else {
            ComparisonResult::Different
        },
        machine_equal: Some(machine_equal),
        continuity_equal: Some(continuity_equal),
        worker_terminal: true,
    }
}
fn valid_sample(sample: &PrivateSample) -> bool {
    matches!(sample.machine_source, 1 | 2)
        && sample.machine_domain == 1
        && (1..=4096).contains(&sample.machine_bytes.len())
        && sample.machine_bytes.iter().any(|byte| *byte != 0)
        && sample.version != 0
        && sample.nonce != [0; 32]
        && [&sample.producer, &sample.domain, &sample.view]
            .into_iter()
            .all(|label| {
                (1..=64).contains(&label.len())
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
}
pub fn encode_sample(sample: &PrivateSample) -> Result<Vec<u8>, ()> {
    if !valid_sample(sample) {
        return Err(());
    }
    let mut payload = vec![sample.machine_source, sample.machine_domain];
    payload.extend_from_slice(&(sample.machine_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(&sample.machine_bytes);
    for label in [&sample.producer, &sample.domain, &sample.view] {
        payload.push(label.len() as u8);
        payload.extend_from_slice(label.as_bytes());
    }
    payload.extend_from_slice(&sample.version.to_le_bytes());
    payload.extend_from_slice(&sample.nonce);
    let mut frame = b"CMP1".to_vec();
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    if frame.len() > MAX_SAMPLE_BYTES {
        Err(())
    } else {
        Ok(frame)
    }
}
pub fn decode_sample(frame: &[u8]) -> Result<PrivateSample, ()> {
    if frame.len() < 8 || frame.len() > MAX_SAMPLE_BYTES || &frame[..4] != b"CMP1" {
        return Err(());
    }
    let length = u32::from_le_bytes(frame[4..8].try_into().map_err(|_| ())?) as usize;
    if length.checked_add(8) != Some(frame.len()) {
        return Err(());
    }
    let mut rest = &frame[8..];
    fn take<'a>(rest: &mut &'a [u8], length: usize) -> Result<&'a [u8], ()> {
        let (value, tail) = rest.split_at_checked(length).ok_or(())?;
        *rest = tail;
        Ok(value)
    }
    let machine_source = take(&mut rest, 1)?[0];
    let machine_domain = take(&mut rest, 1)?[0];
    let count = u32::from_le_bytes(take(&mut rest, 4)?.try_into().map_err(|_| ())?) as usize;
    if !(1..=4096).contains(&count) {
        return Err(());
    }
    let machine_bytes = take(&mut rest, count)?.to_vec();
    fn label(rest: &mut &[u8]) -> Result<String, ()> {
        let length = take(rest, 1)?[0] as usize;
        if !(1..=64).contains(&length) {
            return Err(());
        }
        String::from_utf8(take(rest, length)?.to_vec()).map_err(|_| ())
    }
    let producer = label(&mut rest)?;
    let domain = label(&mut rest)?;
    let view = label(&mut rest)?;
    let version = u32::from_le_bytes(take(&mut rest, 4)?.try_into().map_err(|_| ())?);
    let nonce = take(&mut rest, 32)?.try_into().map_err(|_| ())?;
    let sample = PrivateSample {
        machine_source,
        machine_domain,
        machine_bytes,
        producer,
        version,
        domain,
        view,
        nonce,
    };
    if !rest.is_empty() || !valid_sample(&sample) {
        Err(())
    } else {
        Ok(sample)
    }
}
pub fn sample_handshake(reader: &mut impl std::io::Read, pipes: bool) -> bool {
    if !pipes {
        return false;
    }
    let mut exact = [0; SAMPLE_HANDSHAKE.len()];
    let mut extra = [0];
    reader.read_exact(&mut exact).is_ok()
        && exact == SAMPLE_HANDSHAKE
        && matches!(reader.read(&mut extra), Ok(0))
}
pub fn collect_sample_child(
    child: &mut std::process::Child,
    deadline: std::time::Instant,
    mut on_timeout: impl FnMut(),
) -> Result<PrivateSample, ComparisonResult> {
    use std::io::Read;
    // Readers retain their actual stream in the event until actual child exit.
    enum PipeEnd {
        Output(Result<PrivateSample, ()>, std::process::ChildStdout),
        Error(bool, std::process::ChildStderr),
    }
    let (sender, events) = mpsc::sync_channel(2);
    let mut readers = Vec::new();
    let mut expected = 0;
    if let Some(mut output) = child.stdout.take() {
        expected += 1;
        let sender = sender.clone();
        readers.push(thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0; 256];
            let mut invalid = false;
            loop {
                match output.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        if bytes.len() + count <= MAX_SAMPLE_BYTES {
                            bytes.extend_from_slice(&chunk[..count]);
                        } else {
                            invalid = true;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        invalid = true;
                        break;
                    }
                }
            }
            let reply = if invalid {
                Err(())
            } else {
                decode_sample(&bytes)
            };
            let _ = sender.send(PipeEnd::Output(reply, output));
        }));
    }
    if let Some(mut error) = child.stderr.take() {
        expected += 1;
        let sender = sender.clone();
        readers.push(thread::spawn(move || {
            let mut chunk = [0; 256];
            let mut nonempty = false;
            loop {
                match error.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(_) => nonempty = true,
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        nonempty = true;
                        break;
                    }
                }
            }
            let _ = sender.send(PipeEnd::Error(nonempty, error));
        }));
    }
    drop(sender);
    let mut received = 0;
    let mut retained = Vec::new();
    let mut sample = None;
    let mut invalid = expected != 2;
    let mut timed_out = false;
    let status = loop {
        if !timed_out && std::time::Instant::now() >= deadline {
            timed_out = true;
            on_timeout();
        }
        if received < expected {
            match events.recv_timeout(Duration::from_millis(25)) {
                Ok(event) => {
                    received += 1;
                    match &event {
                        PipeEnd::Output(result, _) => match result {
                            Ok(value) => sample = Some(value.clone()),
                            Err(()) => invalid = true,
                        },
                        PipeEnd::Error(nonempty, _) => invalid |= *nonempty,
                    }
                    retained.push(event);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    invalid = true;
                    received = expected;
                }
            }
        } else if let Ok(Some(status)) = child.try_wait() {
            break status;
        } else {
            thread::sleep(Duration::from_millis(25));
        }
    };
    // EOF/read completion AND actual process completion precede reader disposal.
    let waited = child.wait().is_ok();
    for reader in readers {
        invalid |= reader.join().is_err();
    }
    for event in retained {
        match event {
            PipeEnd::Output(_, stream) => drop(stream),
            PipeEnd::Error(_, stream) => drop(stream),
        }
    }
    if timed_out {
        Err(ComparisonResult::TimedOut)
    } else if invalid || !waited || !status.success() {
        Err(ComparisonResult::Failed)
    } else {
        sample.ok_or(ComparisonResult::Failed)
    }
}

pub fn run_comparison(
    mut start: impl FnMut() -> std::io::Result<std::process::Child>,
    deadline: std::time::Instant,
    mut on_timeout: impl FnMut(),
) -> ComparisonReport {
    let mut samples = Vec::with_capacity(2);
    for ordinal in 0..2 {
        if ordinal != 0 && std::time::Instant::now() >= deadline {
            on_timeout();
            return ComparisonReport::failed(ComparisonResult::TimedOut);
        }
        let mut child = match start() {
            Ok(child) => child,
            Err(_) => return ComparisonReport::failed(ComparisonResult::Failed),
        };
        match collect_sample_child(&mut child, deadline, &mut on_timeout) {
            Ok(sample) => samples.push(sample),
            Err(result) => return ComparisonReport::failed(result),
        }
    }
    compare_samples(&samples[0], &samples[1])
}
