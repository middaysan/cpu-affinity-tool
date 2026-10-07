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
pub struct Failure {
    pub kind: FailureKind,
    pub code: Option<u32>,
}
impl From<TuningNativeError> for Failure {
    fn from(error: TuningNativeError) -> Self {
        let (kind, code) = match error {
            TuningNativeError::Busy => (FailureKind::Busy, None),
            TuningNativeError::Unsupported(_) => (FailureKind::Unsupported, None),
            TuningNativeError::UnsafeStore(_) => (FailureKind::Unsafe, None),
            TuningNativeError::Poisoned => (FailureKind::Poisoned, None),
            TuningNativeError::Capacity { .. } => (FailureKind::Capacity, None),
            TuningNativeError::Denied { code, .. } => (FailureKind::Denied, Some(code)),
            TuningNativeError::Io { code, .. } => (FailureKind::Io, Some(code)),
        };
        Self { kind, code }
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
