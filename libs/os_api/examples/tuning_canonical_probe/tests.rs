use super::*;
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Mutex},
    thread::ThreadId,
};

#[derive(Default)]
struct Evidence {
    calls: Vec<(&'static str, ThreadId)>,
}
struct ContinuityGate {
    entered: SyncSender<()>,
    release: Receiver<()>,
}
struct Fake {
    evidence: Arc<Mutex<Evidence>>,
    token: TokenContext,
    failure: Option<Failure>,
    token_failure: Option<Failure>,
    validate_failure: Option<Failure>,
    read_failure: Option<Failure>,
    canonical: bool,
    presence: Presence,
    continuity_supported: bool,
    continuity_result: Result<ContinuityCheck, Failure>,
    continuity_gate: Option<ContinuityGate>,
    trace_backend_drop: bool,
}
struct FakeStore {
    evidence: Arc<Mutex<Evidence>>,
    canonical: bool,
    presence: Presence,
    validate_failure: Option<Failure>,
    read_failure: Option<Failure>,
    continuity_result: Result<ContinuityCheck, Failure>,
    continuity_gate: Option<ContinuityGate>,
    _worker_only: PhantomData<Rc<()>>,
}
fn record(evidence: &Arc<Mutex<Evidence>>, call: &'static str) {
    evidence
        .lock()
        .unwrap()
        .calls
        .push((call, std::thread::current().id()));
}
impl Backend for Fake {
    type Store = FakeStore;
    fn continuity_supported_build(&self) -> bool {
        record(&self.evidence, "continuity_build");
        self.continuity_supported
    }
    fn token_context(&mut self) -> Result<TokenContext, Failure> {
        record(&self.evidence, "token");
        if let Some(failure) = self.token_failure {
            return Err(failure);
        }
        Ok(self.token)
    }
    fn open_canonical(&mut self) -> Result<Self::Store, Failure> {
        record(&self.evidence, "open");
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        Ok(FakeStore {
            evidence: self.evidence.clone(),
            canonical: self.canonical,
            presence: self.presence,
            validate_failure: self.validate_failure,
            read_failure: self.read_failure,
            continuity_result: self.continuity_result,
            continuity_gate: self.continuity_gate.take(),
            _worker_only: PhantomData,
        })
    }
}
impl ProbeStore for FakeStore {
    fn canonical(&self) -> bool {
        self.canonical
    }
    fn validate(&mut self) -> Result<(), Failure> {
        record(&self.evidence, "validate");
        self.validate_failure.map_or(Ok(()), Err)
    }
    fn presence(&mut self) -> Result<Presence, Failure> {
        record(&self.evidence, "read");
        self.read_failure.map_or(Ok(self.presence), Err)
    }
    fn continuity(&mut self, mode: ContinuityMode) -> Result<ContinuityCheck, Failure> {
        record(
            &self.evidence,
            match mode {
                ContinuityMode::ObserveExisting => "continuity_observe",
                ContinuityMode::InitializeWhenAbsent => "continuity_initialize",
            },
        );
        if let Some(gate) = &self.continuity_gate {
            gate.entered.send(()).unwrap();
            gate.release.recv().unwrap();
        }
        self.continuity_result
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        if self.trace_backend_drop {
            record(&self.evidence, "backend_drop");
        }
    }
}
impl Drop for FakeStore {
    fn drop(&mut self) {
        record(&self.evidence, "drop");
    }
}
fn fake(evidence: Arc<Mutex<Evidence>>, token: TokenContext) -> Fake {
    Fake {
        evidence,
        token,
        failure: None,
        token_failure: None,
        validate_failure: None,
        read_failure: None,
        canonical: true,
        presence: Presence::Absent,
        continuity_supported: true,
        continuity_result: Ok(ContinuityCheck::Stable),
        continuity_gate: None,
        trace_backend_drop: false,
    }
}
fn context(elevation: Elevation, integrity: Integrity) -> TokenContext {
    TokenContext {
        elevation,
        integrity,
    }
}
fn terminal(worker: Worker) -> Report {
    let report = worker.reports.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(report.worker_terminal);
    assert!(worker.join());
    report
}
#[test]
fn commands_default_to_preflight_and_refuse_overrides() {
    assert_eq!(parse_mode(&[]), Ok(Mode::Preflight));
    for (arg, mode) in [
        ("preflight", Mode::Preflight),
        ("lab-open", Mode::LabOpen),
        ("lab-hold", Mode::LabHold),
        ("lab-contend", Mode::LabContend),
    ] {
        assert_eq!(parse_mode(&[arg.into()]), Ok(mode));
    }
    for args in [
        vec!["--root".into()],
        vec!["lab-open".into(), "other-path".into()],
        vec!["publish".into()],
        vec!["".into()],
    ] {
        assert!(parse_mode(&args).is_err());
    }
}
#[test]
fn default_preflight_never_opens_for_any_token() {
    for (token, outcome) in [
        (
            context(Elevation::Standard, Integrity::Medium),
            Outcome::ReadOnlyStandard,
        ),
        (
            context(Elevation::Elevated, Integrity::High),
            Outcome::SkippedElevated,
        ),
        (TokenContext::UNKNOWN, Outcome::UnknownToken),
    ] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let report = terminal(Worker::start(Mode::Preflight, move || fake(source, token)));
        assert_eq!(report.outcome, outcome);
        assert!(!report.store_attempted);
        assert_eq!(
            evidence
                .lock()
                .unwrap()
                .calls
                .iter()
                .map(|x| x.0)
                .collect::<Vec<_>>(),
            vec!["token"]
        );
    }
}
#[test]
fn lab_requires_actual_elevation_and_high_or_system_integrity() {
    for token in [
        context(Elevation::Standard, Integrity::High),
        context(Elevation::Elevated, Integrity::Medium),
        context(Elevation::Elevated, Integrity::Unknown),
        TokenContext::UNKNOWN,
    ] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let report = terminal(Worker::start(Mode::LabOpen, move || fake(source, token)));
        assert!(matches!(
            report.outcome,
            Outcome::LabContextDenied | Outcome::UnknownToken
        ));
        assert!(!report.store_attempted);
        assert_eq!(evidence.lock().unwrap().calls.len(), 1);
    }
}
#[test]
fn lab_open_reads_uninterpreted_bytes_and_drops_on_that_worker() {
    for integrity in [Integrity::High, Integrity::System] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let report = terminal(Worker::start(Mode::LabOpen, move || {
            let mut backend = fake(source, context(Elevation::Elevated, integrity));
            backend.presence = Presence::PresentUninterpreted(999);
            backend
        }));
        assert_eq!(report.outcome, Outcome::Opened);
        assert_eq!(report.journal, Presence::PresentUninterpreted(999));
        let evidence = evidence.lock().unwrap();
        assert_eq!(
            evidence.calls.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec!["token", "open", "validate", "read", "drop"]
        );
        assert!(evidence.calls.iter().all(|x| x.1 == evidence.calls[0].1));
        assert_ne!(evidence.calls[0].1, std::thread::current().id());
    }
}
#[test]
fn contender_requires_busy_and_never_reads_an_unexpected_lease() {
    for busy in [false, true] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let report = terminal(Worker::start(Mode::LabContend, move || {
            let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
            if busy {
                backend.failure = Some(TuningNativeError::Busy.into());
            }
            backend
        }));
        assert_eq!(
            report.outcome,
            if busy {
                Outcome::Busy
            } else {
                Outcome::UnexpectedLeaseAcquired
            }
        );
        assert!(!evidence.lock().unwrap().calls.iter().any(|x| x.0 == "read"));
    }
}
#[test]
fn hold_ready_preserves_lease_until_explicit_release_and_drop() {
    let evidence = Arc::new(Mutex::new(Evidence::default()));
    let source = evidence.clone();
    let worker = Worker::start(Mode::LabHold, move || {
        fake(source, context(Elevation::Elevated, Integrity::High))
    });
    let ready = worker.reports.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(ready.outcome, Outcome::Ready);
    assert!(!ready.worker_terminal);
    assert!(!evidence.lock().unwrap().calls.iter().any(|x| x.0 == "drop"));
    worker.control.send(Control::Release).unwrap();
    let closed = worker.reports.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(closed.worker_terminal);
    assert_eq!(closed.outcome, Outcome::Completed);
    assert_eq!(evidence.lock().unwrap().calls.last().unwrap().0, "drop");
    assert!(worker.join());
}
#[test]
fn noncanonical_authority_is_rejected_before_read() {
    let evidence = Arc::new(Mutex::new(Evidence::default()));
    let source = evidence.clone();
    let report = terminal(Worker::start(Mode::LabOpen, move || {
        let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
        backend.canonical = false;
        backend
    }));
    assert_eq!(report.outcome, Outcome::NonCanonical);
    assert!(!evidence.lock().unwrap().calls.iter().any(|x| x.0 == "read"));
}
#[test]
fn reports_omit_raw_error_strings_and_are_bounded() {
    let mut report = Report::new(Mode::Preflight, Stage::Preflight, Outcome::NativeFailure);
    report.failure = Some(
        TuningNativeError::Denied {
            operation: "S-1-5-21-secret-user E:\\secret\\journal.json",
            code: 5,
        }
        .into(),
    );
    let line = report.line().unwrap();
    assert!(!line.contains("secret"));
    assert!(!line.contains("S-1-5"));
    assert!(line.len() <= MAX_REPORT_BYTES);
    assert!(serde_json::from_str::<serde_json::Value>(&line).is_ok());
}
#[test]
fn control_frame_is_exact_and_input_allocation_is_bounded() {
    use std::io::Cursor;
    for valid in [b"RELEASE\n".as_slice(), b"RELEASE\r\n"] {
        assert_eq!(read_control(&mut Cursor::new(valid)), Control::Release);
    }
    for invalid in [b"RELEASE".as_slice(), b"release\n", b"publish\n", b""] {
        assert_eq!(read_control(&mut Cursor::new(invalid)), Control::Invalid);
    }
    let mut huge = Cursor::new(vec![b'x'; 100_000]);
    assert_eq!(read_control(&mut huge), Control::Invalid);
    assert!(huge.position() <= MAX_CONTROL_BYTES as u64);
}
#[test]
fn integrity_requires_exact_mandatory_sid_and_known_rid() {
    for (rid, expected) in [
        (0u32, Integrity::Untrusted),
        (0x1000, Integrity::Low),
        (0x2000, Integrity::Medium),
        (0x2100, Integrity::Medium),
        (0x3000, Integrity::High),
        (0x4000, Integrity::System),
        (0x5000, Integrity::Unknown),
    ] {
        let mut sid = vec![1, 1, 0, 0, 0, 0, 0, 16];
        sid.extend(rid.to_le_bytes());
        assert_eq!(classify_integrity_sid(&sid), expected);
        for altered in [0usize, 1, 2, 7] {
            let mut malformed = sid.clone();
            malformed[altered] ^= 1;
            assert_eq!(classify_integrity_sid(&malformed), Integrity::Unknown);
        }
        assert_eq!(classify_integrity_sid(&sid[..11]), Integrity::Unknown);
    }
}

#[test]
fn unknown_token_query_never_attempts_any_store() {
    for mode in [
        Mode::Preflight,
        Mode::LabOpen,
        Mode::LabHold,
        Mode::LabContend,
    ] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let report = terminal(Worker::start(mode, move || {
            let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
            backend.token_failure = Some(
                TuningNativeError::Denied {
                    operation: "token",
                    code: 5,
                }
                .into(),
            );
            backend
        }));
        assert_eq!(report.outcome, Outcome::UnknownToken);
        assert!(!report.store_attempted);
        assert_eq!(evidence.lock().unwrap().calls.len(), 1);
    }
}
#[test]
fn authority_and_read_failures_close_same_worker_without_ready() {
    for validation in [false, true] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let report = terminal(Worker::start(Mode::LabHold, move || {
            let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
            let failure =
                Some(TuningNativeError::UnsafeStore("preserve existing unknown bytes").into());
            if validation {
                backend.validate_failure = failure;
            } else {
                backend.read_failure = failure;
            }
            backend
        }));
        assert_eq!(report.outcome, Outcome::NativeFailure);
        assert_eq!(
            report.stage,
            if validation {
                Stage::Validate
            } else {
                Stage::Read
            }
        );
        assert_eq!(
            report.journal,
            if validation {
                Presence::NotRead
            } else {
                Presence::Unreadable
            }
        );
        let evidence = evidence.lock().unwrap();
        assert_eq!(evidence.calls.last().unwrap().0, "drop");
        if validation {
            assert!(!evidence.calls.iter().any(|x| x.0 == "read"));
        }
    }
}
#[test]
fn hold_invalid_or_timeout_closes_without_claiming_release_success() {
    for invalid in [false, true] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = evidence.clone();
        let worker = Worker::start_with_hold_deadline(
            Mode::LabHold,
            move || fake(source, context(Elevation::Elevated, Integrity::High)),
            if invalid {
                Duration::from_secs(2)
            } else {
                Duration::ZERO
            },
        );
        let ready = worker.reports.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ready.outcome, Outcome::Ready);
        if invalid {
            worker.control.send(Control::Invalid).unwrap();
        }
        let closed = worker.reports.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(closed.worker_terminal);
        assert_eq!(
            closed.outcome,
            if invalid {
                Outcome::ControlRejected
            } else {
                Outcome::ControlTimedOut
            }
        );
        assert!(!closed.successful());
        assert_eq!(evidence.lock().unwrap().calls.last().unwrap().0, "drop");
        assert!(worker.join());
    }
}
#[test]
fn oversized_existing_metadata_is_not_a_successful_open() {
    let evidence = Arc::new(Mutex::new(Evidence::default()));
    let report = terminal(Worker::start(Mode::LabOpen, move || {
        let mut backend = fake(evidence, context(Elevation::Elevated, Integrity::High));
        backend.presence = Presence::Oversized(u64::MAX);
        backend
    }));
    assert_eq!(report.journal, Presence::Oversized(u64::MAX));
    assert_eq!(report.outcome, Outcome::NativeFailure);
    assert!(!report.successful());
}

#[test]
fn controller_timeout_or_incomplete_report_stream_never_returns_success() {
    let mut report = Report::new(Mode::LabHold, Stage::Closed, Outcome::Completed);
    report.worker_terminal = true;
    assert_eq!(terminal_exit_code(&report, true, false, true), 0);
    assert_eq!(terminal_exit_code(&report, true, true, true), 2);
    assert_eq!(terminal_exit_code(&report, true, false, false), 2);
    assert_eq!(terminal_exit_code(&report, false, false, true), 2);
    report.worker_terminal = false;
    assert_eq!(terminal_exit_code(&report, true, false, true), 2);
}

fn continuity_run(
    mode: Mode,
    setup: impl FnOnce(&mut Fake) + Send + 'static,
) -> (Report, Arc<Mutex<Evidence>>) {
    let evidence = Arc::new(Mutex::new(Evidence::default()));
    let source = Arc::clone(&evidence);
    let report = terminal(Worker::start(mode, move || {
        let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
        backend.trace_backend_drop = true;
        setup(&mut backend);
        backend
    }));
    (report, evidence)
}
fn calls(evidence: &Arc<Mutex<Evidence>>) -> Vec<&'static str> {
    evidence
        .lock()
        .unwrap()
        .calls
        .iter()
        .map(|call| call.0)
        .collect()
}

#[test]
fn continuity_commands_are_explicit_and_reject_path_or_identity_overrides() {
    for (command, expected) in [
        ("lab-continuity-observe", Mode::LabContinuityObserve),
        ("lab-continuity-initialize", Mode::LabContinuityInitialize),
    ] {
        assert_eq!(parse_mode(&[command.into()]), Ok(expected));
        for extra in ["--root", "E:\\private\\replacement", "--nonce", "--elevate"] {
            assert!(parse_mode(&[command.into(), extra.into()]).is_err());
        }
    }
    for command in [
        "continuity",
        "lab-continuity",
        "initialize",
        "lab-continuity-repair",
    ] {
        assert!(parse_mode(&[command.into()]).is_err());
    }
    assert_eq!(parse_mode(&[]), Ok(Mode::Preflight));
}

#[test]
fn original_modes_never_check_continuity_build_or_invoke_candidate() {
    for mode in [
        Mode::Preflight,
        Mode::LabOpen,
        Mode::LabHold,
        Mode::LabContend,
    ] {
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let source = Arc::clone(&evidence);
        let worker = Worker::start(mode, move || {
            let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
            backend.continuity_supported = false;
            backend
        });
        if mode == Mode::LabHold {
            assert_eq!(
                worker
                    .reports
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .outcome,
                Outcome::Ready
            );
            worker.control.send(Control::Release).unwrap();
        }
        let report = terminal(worker);
        assert_eq!(report.continuity, None);
        assert!(
            !calls(&evidence)
                .iter()
                .any(|call| call.starts_with("continuity_"))
        );
        assert!(!report.line().unwrap().contains("\"continuity\""));
    }
}

#[test]
fn unsupported_continuity_build_stops_before_token_or_store() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        let (report, evidence) = continuity_run(mode, |backend| {
            backend.continuity_supported = false;
            backend.token_failure = Some(
                TuningNativeError::Denied {
                    operation: "must not query token",
                    code: 5,
                }
                .into(),
            );
        });
        assert_eq!(report.outcome, Outcome::UnsupportedBuild);
        assert_eq!(report.stage, Stage::Preflight);
        assert_eq!(report.token, TokenContext::UNKNOWN);
        assert!(!report.store_attempted);
        assert_eq!(report.continuity, None);
        assert_eq!(report.failure, None);
        assert_eq!(calls(&evidence), ["continuity_build", "backend_drop"]);
        assert_eq!(terminal_exit_code(&report, true, false, true), 2);
    }
}

#[test]
fn continuity_standard_unknown_and_token_errors_never_open_store() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        for (token, failure, outcome) in [
            (
                context(Elevation::Standard, Integrity::Medium),
                None,
                Outcome::LabContextDenied,
            ),
            (
                context(Elevation::Standard, Integrity::High),
                None,
                Outcome::LabContextDenied,
            ),
            (
                context(Elevation::Elevated, Integrity::Medium),
                None,
                Outcome::LabContextDenied,
            ),
            (
                context(Elevation::Elevated, Integrity::Unknown),
                None,
                Outcome::UnknownToken,
            ),
            (TokenContext::UNKNOWN, None, Outcome::UnknownToken),
            (
                context(Elevation::Elevated, Integrity::High),
                Some(
                    TuningNativeError::Denied {
                        operation: "unknown retained token",
                        code: 5,
                    }
                    .into(),
                ),
                Outcome::UnknownToken,
            ),
        ] {
            let (report, evidence) = continuity_run(mode, move |backend| {
                backend.token = token;
                backend.token_failure = failure;
            });
            assert_eq!(report.outcome, outcome);
            assert!(!report.store_attempted);
            assert_eq!(report.continuity, None);
            assert_eq!(report.failure, failure);
            assert_eq!(
                calls(&evidence),
                ["continuity_build", "token", "backend_drop"]
            );
            assert_eq!(terminal_exit_code(&report, true, false, true), 2);
        }
    }
}

#[test]
fn continuity_requires_open_authority_validation_and_bounded_presence_first() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        for fault in 0..7 {
            let (report, evidence) = continuity_run(mode, move |backend| match fault {
                0 => backend.failure = Some(TuningNativeError::Busy.into()),
                1 => backend.canonical = false,
                2 => {
                    backend.validate_failure =
                        Some(TuningNativeError::UnsafeStore("unknown lease").into())
                }
                3 => {
                    backend.read_failure = Some(
                        TuningNativeError::Io {
                            operation: "unknown read",
                            code: 32,
                        }
                        .into(),
                    )
                }
                4 => backend.presence = Presence::Oversized(u64::MAX),
                5 => backend.presence = Presence::Unreadable,
                _ => backend.presence = Presence::NotRead,
            });
            assert_eq!(
                report.outcome,
                if fault == 1 {
                    Outcome::NonCanonical
                } else {
                    Outcome::NativeFailure
                }
            );
            assert_eq!(
                report.stage,
                match fault {
                    0 => Stage::Open,
                    1 => Stage::Authority,
                    2 => Stage::Validate,
                    _ => Stage::Read,
                }
            );
            assert_eq!(report.continuity, None);
            let events = calls(&evidence);
            assert_eq!(events[0..3], ["continuity_build", "token", "open"]);
            assert!(
                !events
                    .iter()
                    .any(|call| matches!(*call, "continuity_observe" | "continuity_initialize"))
            );
            assert_eq!(events.last(), Some(&"backend_drop"));
            if fault == 0 {
                assert!(!events.contains(&"drop"));
            } else {
                assert_eq!(events[events.len() - 2], "drop");
            }
            assert_eq!(terminal_exit_code(&report, true, false, true), 2);
        }
    }
}

#[test]
fn continuity_initialize_refuses_every_present_journal_even_zero_length() {
    for length in [0, 1, 999, u64::MAX] {
        let (report, evidence) = continuity_run(Mode::LabContinuityInitialize, move |backend| {
            backend.presence = Presence::PresentUninterpreted(length);
        });
        assert_eq!(report.outcome, Outcome::ContinuityInitializationBlocked);
        assert_eq!(report.stage, Stage::Read);
        assert_eq!(report.journal, Presence::PresentUninterpreted(length));
        assert_eq!(report.continuity, None);
        assert_eq!(report.failure, None);
        assert_eq!(
            calls(&evidence),
            [
                "continuity_build",
                "token",
                "open",
                "validate",
                "read",
                "drop",
                "backend_drop"
            ]
        );
        assert_eq!(terminal_exit_code(&report, true, false, true), 2);
    }
}

#[test]
fn continuity_stable_summary_requires_explicit_mode_and_same_worker_drop_order() {
    for (mode, presence, intent) in [
        (
            Mode::LabContinuityObserve,
            Presence::Absent,
            "continuity_observe",
        ),
        (
            Mode::LabContinuityObserve,
            Presence::PresentUninterpreted(0),
            "continuity_observe",
        ),
        (
            Mode::LabContinuityObserve,
            Presence::PresentUninterpreted(999),
            "continuity_observe",
        ),
        (
            Mode::LabContinuityInitialize,
            Presence::Absent,
            "continuity_initialize",
        ),
    ] {
        let (report, evidence) = continuity_run(mode, move |backend| {
            backend.presence = presence;
        });
        assert_eq!(report.stage, Stage::Continuity);
        assert_eq!(report.outcome, Outcome::ContinuityStable);
        assert_eq!(report.continuity, Some(ContinuityCheck::Stable));
        assert_eq!(report.journal, presence);
        assert_eq!(report.failure, None);
        assert_eq!(
            calls(&evidence),
            [
                "continuity_build",
                "token",
                "open",
                "validate",
                "read",
                intent,
                "drop",
                "backend_drop"
            ]
        );
        let evidence = evidence.lock().unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|call| call.1 == evidence.calls[0].1)
        );
        assert_ne!(evidence.calls[0].1, std::thread::current().id());
        assert_eq!(terminal_exit_code(&report, true, false, true), 0);
    }
}

#[test]
fn continuity_unknown_changed_and_native_errors_never_report_success() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        for (check, outcome) in [
            (ContinuityCheck::Unknown, Outcome::ContinuityUnknown),
            (ContinuityCheck::Changed, Outcome::ContinuityChanged),
        ] {
            let (report, evidence) = continuity_run(mode, move |backend| {
                backend.continuity_result = Ok(check);
            });
            assert_eq!(report.stage, Stage::Continuity);
            assert_eq!(report.outcome, outcome);
            assert_eq!(report.continuity, Some(check));
            assert_eq!(report.failure, None);
            assert_eq!(calls(&evidence).last(), Some(&"backend_drop"));
            assert_eq!(terminal_exit_code(&report, true, false, true), 2);
        }
        let failure: Failure = TuningNativeError::Denied {
            operation: "private SID S-1-5-21 raw nonce E:\\secret\\metadata",
            code: 5,
        }
        .into();
        let (report, evidence) = continuity_run(mode, move |backend| {
            backend.continuity_result = Err(failure);
        });
        assert_eq!(report.stage, Stage::Continuity);
        assert_eq!(report.outcome, Outcome::NativeFailure);
        assert_eq!(report.continuity, None);
        assert_eq!(report.failure, Some(failure));
        assert_eq!(calls(&evidence).last(), Some(&"backend_drop"));
        assert_eq!(terminal_exit_code(&report, true, false, true), 2);
        assert!(!report.line().unwrap().contains("secret"));
    }
}

#[test]
fn continuity_pair_comparison_preserves_full_shape_and_last_nonce_byte() {
    // Private synthetic comparison values, never native Data or a guard.
    #[derive(Clone, PartialEq, Eq)]
    struct Shape {
        producer: u32,
        version: u32,
        domain: u32,
        view: u32,
        nonce: [u8; 32],
    }
    let original = Shape {
        producer: 1,
        version: 1,
        domain: 1,
        view: 1,
        nonce: [0x31; 32],
    };
    assert_eq!(
        continuity_check([Some(original.clone()), Some(original.clone())]),
        ContinuityCheck::Stable
    );
    for field in 0..5 {
        let mut changed = original.clone();
        match field {
            0 => changed.producer += 1,
            1 => changed.version += 1,
            2 => changed.domain += 1,
            3 => changed.view += 1,
            _ => changed.nonce[31] ^= 1,
        }
        assert_eq!(
            continuity_check([Some(original.clone()), Some(changed)]),
            ContinuityCheck::Changed
        );
    }
    assert_eq!(
        continuity_check([None, Some(original.clone())]),
        ContinuityCheck::Unknown
    );
    assert_eq!(
        continuity_check([Some(original), None]),
        ContinuityCheck::Unknown
    );
    assert_eq!(
        continuity_check::<Shape>([None, None]),
        ContinuityCheck::Unknown
    );
}

#[test]
fn new_continuity_success_requires_exact_mode_outcome_and_stable_field() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        let mut report = Report::new(mode, Stage::Continuity, Outcome::ContinuityStable);
        report.continuity = Some(ContinuityCheck::Stable);
        assert!(report.successful());
        for check in [
            None,
            Some(ContinuityCheck::Unknown),
            Some(ContinuityCheck::Changed),
        ] {
            report.continuity = check;
            assert!(!report.successful());
        }
        for outcome in [
            Outcome::Opened,
            Outcome::Completed,
            Outcome::Busy,
            Outcome::SkippedElevated,
            Outcome::ReadOnlyStandard,
            Outcome::ContinuityUnknown,
            Outcome::ContinuityChanged,
            Outcome::NativeFailure,
        ] {
            report.outcome = outcome;
            report.continuity = Some(ContinuityCheck::Stable);
            assert!(!report.successful());
        }
    }
    let mut wrong_mode = Report::new(Mode::LabOpen, Stage::Continuity, Outcome::ContinuityStable);
    wrong_mode.continuity = Some(ContinuityCheck::Stable);
    assert!(!wrong_mode.successful());
}

#[test]
fn continuity_none_preserves_old_wire_and_new_reports_are_private_and_bounded() {
    let mut original = Report::new(Mode::Preflight, Stage::Preflight, Outcome::ReadOnlyStandard);
    original.token = context(Elevation::Standard, Integrity::Medium);
    original.worker_terminal = true;
    assert_eq!(
        original.line().unwrap(),
        r#"{"schema":1,"command":"preflight","stage":"preflight","outcome":"read_only_standard","token":{"elevation":"standard","integrity":"medium"},"store_attempted":false,"authority":"not_opened","journal":{"kind":"not_read"},"failure":null,"worker_terminal":true}"#
    );
    for check in [
        ContinuityCheck::Stable,
        ContinuityCheck::Unknown,
        ContinuityCheck::Changed,
    ] {
        let mut report = Report::new(
            Mode::LabContinuityObserve,
            Stage::Continuity,
            Outcome::NativeFailure,
        );
        report.continuity = Some(check);
        report.journal = Presence::PresentUninterpreted(u64::MAX);
        report.failure = Some(
            TuningNativeError::Io {
                operation: "raw private machine SID nonce E:\\private\\store",
                code: u32::MAX,
            }
            .into(),
        );
        let line = report.line().unwrap();
        assert!(line.len() <= MAX_REPORT_BYTES);
        assert!(!line.contains('\n'));
        assert!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["continuity"].is_string()
        );
        for private in ["raw", "private", "machine", "SID", "nonce", "E:\\"] {
            assert!(!line.contains(private));
        }
    }
}

#[test]
fn continuity_terminal_success_cannot_clear_controller_timeout_or_output_failure() {
    let mut report = Report::new(
        Mode::LabContinuityObserve,
        Stage::Continuity,
        Outcome::ContinuityStable,
    );
    report.continuity = Some(ContinuityCheck::Stable);
    report.worker_terminal = true;
    assert_eq!(terminal_exit_code(&report, true, false, true), 0);
    assert_eq!(terminal_exit_code(&report, true, true, true), 2);
    assert_eq!(terminal_exit_code(&report, true, false, false), 2);
    assert_eq!(terminal_exit_code(&report, false, false, true), 2);
    report.worker_terminal = false;
    assert_eq!(terminal_exit_code(&report, true, false, true), 2);
}

#[test]
fn continuity_timeout_retains_the_same_worker_until_explicit_completion() {
    let evidence = Arc::new(Mutex::new(Evidence::default()));
    let source = Arc::clone(&evidence);
    let (entered, entered_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::sync_channel(1);
    let worker = Worker::start(Mode::LabContinuityObserve, move || {
        let mut backend = fake(source, context(Elevation::Elevated, Integrity::High));
        backend.trace_backend_drop = true;
        backend.continuity_gate = Some(ContinuityGate {
            entered,
            release: release_rx,
        });
        backend
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        worker.reports.recv_timeout(Duration::ZERO),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        calls(&evidence),
        [
            "continuity_build",
            "token",
            "open",
            "validate",
            "read",
            "continuity_observe"
        ]
    );
    release.send(()).unwrap();
    let report = terminal(worker);
    assert_eq!(report.outcome, Outcome::ContinuityStable);
    assert_eq!(report.continuity, Some(ContinuityCheck::Stable));
    assert_eq!(
        calls(&evidence),
        [
            "continuity_build",
            "token",
            "open",
            "validate",
            "read",
            "continuity_observe",
            "drop",
            "backend_drop"
        ]
    );
    let evidence = evidence.lock().unwrap();
    assert!(
        evidence
            .calls
            .iter()
            .all(|call| call.1 == evidence.calls[0].1)
    );
    assert_ne!(evidence.calls[0].1, std::thread::current().id());
    assert_eq!(terminal_exit_code(&report, true, true, true), 2);
}

const STORE_REFUSALS: [(&str, StoreRefusal, &str); 20] = [
    (
        "untrusted object owner",
        StoreRefusal::OwnerUntrusted,
        "owner_untrusted",
    ),
    ("missing DACL", StoreRefusal::DaclMissing, "dacl_missing"),
    (
        "unprotected managed DACL",
        StoreRefusal::DaclUnprotected,
        "dacl_unprotected",
    ),
    ("empty DACL", StoreRefusal::DaclEmpty, "dacl_empty"),
    (
        "untrusted storage mutation grant",
        StoreRefusal::MutationGrantUntrusted,
        "mutation_grant_untrusted",
    ),
    (
        "missing authority grant",
        StoreRefusal::AuthorityGrantMissing,
        "authority_grant_missing",
    ),
    (
        "missing high integrity no-write-up",
        StoreRefusal::IntegrityLabelInvalid,
        "integrity_label_invalid",
    ),
    ("missing ACL", StoreRefusal::AclMissing, "acl_missing"),
    ("malformed ACE", StoreRefusal::AceMalformed, "ace_malformed"),
    (
        "malformed ACE SID",
        StoreRefusal::AceSidMalformed,
        "ace_sid_malformed",
    ),
    (
        "unexpected object type/reparse/hardlink",
        StoreRefusal::ObjectTopologyInvalid,
        "object_topology_invalid",
    ),
    (
        "held directory changed",
        StoreRefusal::ObjectChanged,
        "object_changed",
    ),
    (
        "held lease changed",
        StoreRefusal::ObjectChanged,
        "object_changed",
    ),
    (
        "directory namespace changed",
        StoreRefusal::NamespaceChanged,
        "namespace_changed",
    ),
    (
        "lease namespace changed",
        StoreRefusal::NamespaceChanged,
        "namespace_changed",
    ),
    (
        "unknown managed artifacts",
        StoreRefusal::ArtifactsUnexpected,
        "artifacts_unexpected",
    ),
    (
        "pending or unknown journal artifact",
        StoreRefusal::ArtifactsUnexpected,
        "artifacts_unexpected",
    ),
    (
        "absolute store required",
        StoreRefusal::PathInvalid,
        "path_invalid",
    ),
    (
        "normalized store components required",
        StoreRefusal::PathInvalid,
        "path_invalid",
    ),
    (
        "invalid native path",
        StoreRefusal::PathInvalid,
        "path_invalid",
    ),
];

#[test]
fn store_refusals_report_only_the_exact_allowlisted_predicate() {
    for (reason, expected, wire) in STORE_REFUSALS {
        let failure = Failure::from(TuningNativeError::UnsafeStore(reason));
        assert_eq!(failure.kind, FailureKind::Unsafe);
        assert_eq!(failure.code, None);
        assert_eq!(failure.store_refusal, Some(expected));
        assert_eq!(
            serde_json::to_value(failure).unwrap()["store_refusal"],
            wire
        );
    }
}

#[test]
fn unrecognized_private_refusal_text_and_lookalikes_remain_omitted() {
    for reason in [
        "",
        "S-1-5-21-private-user E:\\private\\store nonce=private",
        "untrusted object owner E:\\private\\store",
        "prefix untrusted object owner",
        "UNTRUSTED OBJECT OWNER",
        "untrusted object owner\n",
        "missing DACL\r\nS-1-5-21-private-user",
        "canonical continuity candidate requires canonical store",
    ] {
        let failure = Failure::from(TuningNativeError::UnsafeStore(reason));
        assert_eq!(failure.kind, FailureKind::Unsafe);
        assert_eq!(failure.code, None);
        assert_eq!(failure.store_refusal, None);
        assert_eq!(
            serde_json::to_string(&failure).unwrap(),
            r#"{"kind":"unsafe","code":null}"#
        );
    }
}

#[test]
fn store_refusal_mapping_never_reclassifies_other_native_errors() {
    for (error, kind, code) in [
        (TuningNativeError::Busy, FailureKind::Busy, None),
        (TuningNativeError::Poisoned, FailureKind::Poisoned, None),
        (
            TuningNativeError::Unsupported("untrusted object owner"),
            FailureKind::Unsupported,
            None,
        ),
        (
            TuningNativeError::Capacity {
                size: u64::MAX,
                limit: usize::MAX,
            },
            FailureKind::Capacity,
            None,
        ),
        (
            TuningNativeError::Denied {
                operation: "untrusted object owner",
                code: 5,
            },
            FailureKind::Denied,
            Some(5),
        ),
        (
            TuningNativeError::Io {
                operation: "missing DACL E:\\private\\store",
                code: u32::MAX,
            },
            FailureKind::Io,
            Some(u32::MAX),
        ),
    ] {
        let failure = Failure::from(error);
        assert_eq!(failure.kind, kind);
        assert_eq!(failure.code, code);
        assert_eq!(failure.store_refusal, None);
        assert!(
            serde_json::to_value(failure)
                .unwrap()
                .get("store_refusal")
                .is_none()
        );
        assert!(!serde_json::to_string(&failure).unwrap().contains("private"));
    }
}

#[test]
fn canonical_open_refusal_remains_terminal_without_continuity_or_cleanup_claim() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        let (report, evidence) = continuity_run(mode, |backend| {
            backend.failure =
                Some(TuningNativeError::UnsafeStore("untrusted storage mutation grant").into());
        });
        assert_eq!(report.stage, Stage::Open);
        assert_eq!(report.outcome, Outcome::NativeFailure);
        assert!(report.store_attempted);
        assert_eq!(report.authority, Authority::NotOpened);
        assert_eq!(report.journal, Presence::NotRead);
        assert_eq!(report.continuity, None);
        assert!(report.worker_terminal);
        assert_eq!(
            calls(&evidence),
            ["continuity_build", "token", "open", "backend_drop"]
        );
        let failure = report.failure.unwrap();
        assert_eq!(failure.kind, FailureKind::Unsafe);
        assert_eq!(failure.code, None);
        assert_eq!(
            failure.store_refusal,
            Some(StoreRefusal::MutationGrantUntrusted)
        );
        let evidence = evidence.lock().unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|call| call.1 == evidence.calls[0].1)
        );
        assert_ne!(evidence.calls[0].1, std::thread::current().id());
        assert_eq!(terminal_exit_code(&report, true, false, true), 2);
    }
}

#[test]
fn refusal_reports_are_bounded_fixed_enums_and_none_keeps_old_failure_wire() {
    let mut report = Report::new(Mode::LabOpen, Stage::Open, Outcome::NativeFailure);
    report.failure = Some(TuningNativeError::UnsafeStore("unrecognized private context").into());
    assert_eq!(
        report.line().unwrap(),
        r#"{"schema":1,"command":"lab_open","stage":"open","outcome":"native_failure","token":{"elevation":"unknown","integrity":"unknown"},"store_attempted":false,"authority":"not_opened","journal":{"kind":"not_read"},"failure":{"kind":"unsafe","code":null},"worker_terminal":false}"#
    );
    for (reason, _, wire) in STORE_REFUSALS {
        report.failure = Some(TuningNativeError::UnsafeStore(reason).into());
        let line = report.line().unwrap();
        assert!(line.len() <= MAX_REPORT_BYTES);
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["failure"]["store_refusal"],
            wire
        );
        for private in ["E:\\", "S-1-5", "nonce", "private", reason] {
            assert!(!line.contains(private));
        }
        assert_eq!(terminal_exit_code(&report, true, false, true), 2);
    }
}

const CONTINUITY_REFUSALS: [(&str, ContinuityRefusal, &str); 10] = [
    (
        "canonical continuity malformed capsule",
        ContinuityRefusal::MalformedCapsule,
        "malformed_capsule",
    ),
    (
        "canonical continuity query bounds",
        ContinuityRefusal::QueryBound,
        "query_bound",
    ),
    (
        "canonical continuity unsafe security",
        ContinuityRefusal::UnsafeSecurity,
        "unsafe_security",
    ),
    (
        "canonical continuity context unproven",
        ContinuityRefusal::ContextUnproven,
        "context_unproven",
    ),
    (
        "canonical continuity missing component",
        ContinuityRefusal::MissingComponent,
        "missing_component",
    ),
    (
        "canonical continuity initialization refused",
        ContinuityRefusal::InitializationRefused,
        "initialization_refused",
    ),
    (
        "canonical continuity observation unknown",
        ContinuityRefusal::ObservationUnknown,
        "observation_unknown",
    ),
    (
        "publisher domain is not proven unpackaged",
        ContinuityRefusal::PublisherDomainUnproven,
        "publisher_domain_unproven",
    ),
    (
        "system ID source is unavailable",
        ContinuityRefusal::MachineSourceUnavailable,
        "machine_source_unavailable",
    ),
    (
        "system ID byte length",
        ContinuityRefusal::MachineBytesInvalid,
        "machine_bytes_invalid",
    ),
];

#[test]
fn continuity_refusals_require_exact_allowlisted_unsupported_literals() {
    for (reason, expected, wire) in CONTINUITY_REFUSALS {
        let failure = Failure::from(TuningNativeError::Unsupported(reason));
        assert_eq!(failure.kind, FailureKind::Unsupported);
        assert_eq!(failure.code, None);
        assert_eq!(failure.store_refusal, None);
        assert_eq!(failure.continuity_refusal, Some(expected));
        assert_eq!(
            serde_json::to_value(failure).unwrap()["continuity_refusal"],
            wire
        );
    }
}

#[test]
fn continuity_refusal_lookalikes_private_text_and_other_error_classes_are_omitted() {
    for reason in [
        "",
        "S-1-5-21-private-user E:\\private\\store nonce=private",
        "prefix canonical continuity context unproven",
        "canonical continuity context unproven E:\\private\\store",
        "CANONICAL CONTINUITY UNSAFE SECURITY",
        "canonical continuity missing component\n",
        "canonical continuity query bounds\r\nS-1-5-21-private-user",
        "canonical continuity initialization refused ",
        "canonical continuity malformed capsule: private",
        " canonical continuity observation unknown",
        "publisher domain is not proven unpackaged\n",
        "system ID source is unavailable: private",
        "system ID byte length ",
    ] {
        let failure = Failure::from(TuningNativeError::Unsupported(reason));
        assert_eq!(failure.continuity_refusal, None);
        assert_eq!(
            serde_json::to_string(&failure).unwrap(),
            r#"{"kind":"unsupported","code":null}"#
        );
    }
    for (reason, _, _) in CONTINUITY_REFUSALS {
        for (error, kind, code) in [
            (
                TuningNativeError::UnsafeStore(reason),
                FailureKind::Unsafe,
                None,
            ),
            (
                TuningNativeError::Denied {
                    operation: reason,
                    code: 5,
                },
                FailureKind::Denied,
                Some(5),
            ),
            (
                TuningNativeError::Io {
                    operation: reason,
                    code: u32::MAX,
                },
                FailureKind::Io,
                Some(u32::MAX),
            ),
        ] {
            let failure = Failure::from(error);
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.code, code);
            assert_eq!(failure.store_refusal, None);
            assert_eq!(failure.continuity_refusal, None);
            let value = serde_json::to_value(failure).unwrap();
            assert!(value.get("continuity_refusal").is_none());
            assert!(!serde_json::to_string(&failure).unwrap().contains(reason));
        }
    }
}

#[test]
fn continuity_refusal_errors_remain_terminal_failures_with_same_worker_drop_order() {
    for mode in [Mode::LabContinuityObserve, Mode::LabContinuityInitialize] {
        for (reason, expected, _) in CONTINUITY_REFUSALS {
            let (report, evidence) = continuity_run(mode, move |backend| {
                backend.continuity_result = Err(TuningNativeError::Unsupported(reason).into());
            });
            assert_eq!(report.stage, Stage::Continuity);
            assert_eq!(report.outcome, Outcome::NativeFailure);
            assert_eq!(report.token, context(Elevation::Elevated, Integrity::High));
            assert!(report.store_attempted);
            assert_eq!(report.authority, Authority::Canonical);
            assert_eq!(report.journal, Presence::Absent);
            assert_eq!(report.continuity, None);
            assert!(report.worker_terminal);
            assert!(!report.successful());
            assert_eq!(terminal_exit_code(&report, true, false, true), 2);
            assert_eq!(
                calls(&evidence),
                [
                    "continuity_build",
                    "token",
                    "open",
                    "validate",
                    "read",
                    if mode == Mode::LabContinuityObserve {
                        "continuity_observe"
                    } else {
                        "continuity_initialize"
                    },
                    "drop",
                    "backend_drop",
                ]
            );
            let evidence = evidence.lock().unwrap();
            assert!(
                evidence
                    .calls
                    .iter()
                    .all(|call| call.1 == evidence.calls[0].1)
            );
            assert_ne!(evidence.calls[0].1, std::thread::current().id());
            let failure = report.failure.unwrap();
            assert_eq!(failure.kind, FailureKind::Unsupported);
            assert_eq!(failure.code, None);
            assert_eq!(failure.store_refusal, None);
            assert_eq!(failure.continuity_refusal, Some(expected));
        }
    }
}

#[test]
fn continuity_refusal_wire_is_finite_bounded_and_preserves_existing_failure_shapes() {
    assert_eq!(
        serde_json::to_string(&Failure::from(TuningNativeError::Unsupported(
            "private unknown context"
        )))
        .unwrap(),
        r#"{"kind":"unsupported","code":null}"#,
    );
    for (reason, _, wire) in STORE_REFUSALS {
        let failure = Failure::from(TuningNativeError::UnsafeStore(reason));
        assert_eq!(failure.continuity_refusal, None);
        assert_eq!(
            serde_json::to_value(failure).unwrap(),
            serde_json::json!({
                "kind": "unsafe", "code": null, "store_refusal": wire,
            })
        );
    }
    for (reason, _, wire) in CONTINUITY_REFUSALS {
        let mut report = Report::new(
            Mode::LabContinuityInitialize,
            Stage::Continuity,
            Outcome::NativeFailure,
        );
        report.token = context(Elevation::Elevated, Integrity::High);
        report.store_attempted = true;
        report.authority = Authority::Canonical;
        report.journal = Presence::Absent;
        report.failure = Some(TuningNativeError::Unsupported(reason).into());
        report.worker_terminal = true;
        let line = report.line().unwrap();
        assert!(line.len() <= MAX_REPORT_BYTES);
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["failure"]["continuity_refusal"],
            wire
        );
        for private in ["E:\\", "S-1-5", "nonce", "private", reason] {
            assert!(!line.contains(private));
        }
        assert_eq!(report.continuity, None);
        assert_eq!(terminal_exit_code(&report, true, false, true), 2);
    }
}

fn comparator_shape() -> PrivateSample {
    PrivateSample {
        machine_source: 1,
        machine_domain: 1,
        machine_bytes: vec![0x35; 4096],
        producer: "windows_hklm_software_volatile_marker".into(),
        version: 1,
        domain: "cpu_affinity_tool_tuning".into(),
        view: "native64".into(),
        nonce: [0x5a; 32],
    }
}
#[test]
fn comparator_command_and_handshake_are_explicit_and_pipe_only() {
    assert_eq!(
        parse_mode(&["lab-continuity-compare".into()]),
        Ok(Mode::LabContinuityCompare)
    );
    for args in [
        vec![PRIVATE_SAMPLE_ARGUMENT.into()],
        vec!["lab-continuity-compare".into(), "override".into()],
    ] {
        assert!(parse_mode(&args).is_err());
    }
    assert!(sample_handshake(&mut &SAMPLE_HANDSHAKE[..], true));
    for mut input in [
        b"".as_slice(),
        b"CMP-SAMPLE-1\r\n",
        b"cmp-sample-1\n",
        b"CMP-SAMPLE-1\nextra",
    ] {
        assert!(!sample_handshake(&mut input, true));
    }
    assert!(!sample_handshake(&mut &SAMPLE_HANDSHAKE[..], false));
}
#[test]
fn comparator_full_machine_and_continuity_tuple_is_compared_without_truncation() {
    let first = comparator_shape();
    let equal = compare_samples(&first, &first);
    assert_eq!(equal.result, ComparisonResult::Equal);
    assert_eq!(equal.exit_code(), 0);
    for field in 0..9 {
        let mut second = first.clone();
        match field {
            0 => second.machine_source = 2,
            1 => second.machine_domain = 2,
            2 => second.machine_bytes[4095] ^= 1,
            3 => second.producer.push('x'),
            4 => second.version += 1,
            5 => second.domain.push('x'),
            6 => second.view = "native32".into(),
            7 => second.nonce[31] ^= 1,
            _ => second.machine_bytes.pop().map(|_| ()).unwrap(),
        }
        let report = compare_samples(&first, &second);
        assert_eq!(report.result, ComparisonResult::Different);
        assert_eq!(report.machine_equal, Some((3..=7).contains(&field)));
        assert_eq!(report.continuity_equal, Some(field <= 2 || field == 8));
        assert_eq!(report.exit_code(), 2);
        let line = report.line().unwrap();
        assert!(line.len() < MAX_REPORT_BYTES);
        assert!(
            !line.contains("nonce") && !line.contains("windows_hklm") && !line.contains("5a5a")
        );
    }
    assert!(!format!("{first:?}").contains("windows_hklm"));
    let mut invalid = equal;
    invalid.worker_terminal = false;
    assert_eq!(invalid.exit_code(), 2);
}
#[test]
fn comparator_frame_is_one_exact_bounded_record_and_rejects_malformed_input() {
    let expected = comparator_shape();
    let frame = encode_sample(&expected).unwrap();
    assert!(frame.len() <= MAX_SAMPLE_BYTES);
    assert!(
        decode_sample(&frame).unwrap() == expected,
        "complete private tuple roundtrip"
    );
    for end in [0, 1, 7, 8, frame.len() - 1] {
        assert!(decode_sample(&frame[..end]).is_err());
    }
    let mut extra = frame.clone();
    extra.push(0);
    assert!(decode_sample(&extra).is_err());
    let mut extra = frame.clone();
    extra.extend_from_slice(&frame);
    assert!(decode_sample(&extra).is_err());
    let mut wrong = frame.clone();
    wrong[0] ^= 1;
    assert!(decode_sample(&wrong).is_err());
    let mut wrong = frame.clone();
    wrong[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_sample(&wrong).is_err());
    for field in 0..6 {
        let mut malformed = expected.clone();
        match field {
            0 => malformed.machine_bytes.clear(),
            1 => malformed.machine_bytes.push(1),
            2 => malformed.nonce = [0; 32],
            3 => malformed.producer = "x".repeat(65),
            4 => malformed.domain = "private\nvalue".into(),
            _ => malformed.machine_source = 0,
        }
        assert!(encode_sample(&malformed).is_err());
    }
}

// This child is fixture transport/lifetime only: it never calls native sampling.
#[cfg(windows)]
#[test]
fn comparator_child_fixture() {
    use std::io::{Read, Write};
    let Ok(role) = std::env::var("TUNING_COMPARATOR_FIXTURE_ROLE") else {
        return;
    };
    std::panic::set_hook(Box::new(|_| {}));
    let mut input = std::io::stdin().lock();
    let mut handshake = [0; SAMPLE_HANDSHAKE.len()];
    input.read_exact(&mut handshake).unwrap();
    assert!(handshake == SAMPLE_HANDSHAKE, "fixture handshake");
    let mut output = std::io::stdout().lock();
    output.write_all(b"\nFIXTURE-PIPE\n").unwrap();
    output.flush().unwrap();
    // Fixed fixture framing independent of the implementation encoder.
    let mut payload = vec![1, 1];
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.push(0x35);
    for label in [
        "windows_hklm_software_volatile_marker",
        "cpu_affinity_tool_tuning",
        "native64",
    ] {
        payload.push(label.len() as u8);
        payload.extend_from_slice(label.as_bytes());
    }
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&[0x5a; 32]);
    let mut frame = b"CMP1".to_vec();
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    match role.as_str() {
        "truncated" => output.write_all(&frame[..frame.len() - 1]).unwrap(),
        "extra" => {
            output.write_all(&frame).unwrap();
            output.write_all(b"extra").unwrap();
        }
        _ => output.write_all(&frame).unwrap(),
    }
    output.flush().unwrap();
    drop(output);
    if role == "blocked" {
        let mut release = [0];
        input.read_exact(&mut release).unwrap();
        assert_eq!(release, [1]);
    }
    std::process::exit(if role == "nonzero" { 2 } else { 0 });
}
#[cfg(windows)]
fn comparator_fixture(role: &str) -> std::process::Child {
    use std::io::{Read, Write};
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "logic::tests::comparator_child_fixture",
            "--nocapture",
            "--quiet",
        ])
        .env("TUNING_COMPARATOR_FIXTURE_ROLE", role)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let sent = child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(SAMPLE_HANDSHAKE)
        .is_ok();
    // Strip only bounded libtest setup output, before the production collector.
    let mut stdout = child.stdout.take().unwrap();
    let (ready, result) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut setup = Vec::new();
        let mut byte = [0];
        let valid = loop {
            if setup.ends_with(b"\nFIXTURE-PIPE\n") {
                break true;
            }
            if setup.len() >= 512 || stdout.read_exact(&mut byte).is_err() {
                break false;
            }
            setup.push(byte[0]);
        };
        let _ = ready.send((valid, stdout));
    });
    let prepared = if sent {
        result.recv_timeout(Duration::from_secs(3)).ok()
    } else {
        None
    };
    let Some((true, stdout)) = prepared else {
        // Only the exact transport-fixture child may be terminated on setup failure.
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        panic!("owned fixture pipe setup failed");
    };
    reader.join().unwrap();
    child.stdout = Some(stdout);
    child
}
#[cfg(windows)]
#[test]
fn comparator_owned_child_requires_exact_frame_eof_and_actual_zero_exit() {
    for role in ["complete", "truncated", "extra", "nonzero"] {
        let mut child = comparator_fixture(role);
        let result = collect_sample_child(
            &mut child,
            std::time::Instant::now() + Duration::from_secs(3),
            || {},
        );
        // Cleanup always reaches the exact child before any assertion.
        assert!(child.wait().is_ok());
        assert_eq!(
            result.is_ok(),
            role == "complete",
            "owned fixture terminal classification"
        );
    }
}
#[cfg(windows)]
#[test]
fn comparator_timeout_retains_same_child_and_terminal_looking_frame_until_exit_and_eof() {
    use std::io::Write;
    let mut child = comparator_fixture("blocked");
    let mut release = child.stdin.take().unwrap();
    let (timed_out, notice) = mpsc::sync_channel(1);
    let (returned, result) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let captured = collect_sample_child(&mut child, std::time::Instant::now(), || {
            let _ = timed_out.try_send(());
        });
        let _ = returned.send(captured);
        child.wait().unwrap()
    });
    let timeout = notice.recv_timeout(Duration::from_secs(3));
    let early = result.try_recv();
    // Release exact owned fixture even when an assertion will fail on the stub.
    release.write_all(&[1]).unwrap();
    drop(release);
    let terminal = handle.join().unwrap();
    assert!(terminal.success());
    assert!(
        timeout.is_ok(),
        "timeout must be reported before actual fixture exit"
    );
    assert!(
        matches!(early, Err(mpsc::TryRecvError::Empty)),
        "same child must remain retained until terminal+EOF"
    );
    assert_eq!(
        result.recv().unwrap().unwrap_err(),
        ComparisonResult::TimedOut
    );
}

#[cfg(windows)]
#[test]
fn comparator_serial_controller_starts_second_only_after_first_terminal_success() {
    for role in ["complete", "truncated", "nonzero"] {
        let mut starts = 0;
        let report = run_comparison(
            || {
                starts += 1;
                Ok(comparator_fixture(role))
            },
            std::time::Instant::now() + Duration::from_secs(3),
            || {},
        );
        assert_eq!(starts, if role == "complete" { 2 } else { 1 });
        assert_eq!(report.exit_code(), if role == "complete" { 0 } else { 2 });
    }
}
#[cfg(windows)]
#[test]
fn comparator_serial_controller_never_replaces_timed_out_first_actor() {
    use std::io::Write;
    let mut child = comparator_fixture("blocked");
    let mut release = child.stdin.take().unwrap();
    let slot = Arc::new(Mutex::new(Some(child)));
    let retained = slot.clone();
    let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let actual_starts = starts.clone();
    let (timeout, entered) = mpsc::sync_channel(1);
    let (completed, returned) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let report = run_comparison(
            || {
                actual_starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(retained
                    .lock()
                    .unwrap()
                    .take()
                    .expect("only the first owned fixture"))
            },
            std::time::Instant::now(),
            || {
                let _ = timeout.try_send(());
            },
        );
        let _ = completed.send(report);
    });
    let observed_timeout = entered.recv_timeout(Duration::from_secs(3));
    let early = returned.try_recv();
    let count_before_release = starts.load(std::sync::atomic::Ordering::SeqCst);
    release.write_all(&[1]).unwrap();
    drop(release);
    handle.join().unwrap();
    // Intentional controller stub may leave setup child unstarted; clean it exactly.
    if let Some(mut unused) = slot.lock().unwrap().take() {
        unused.wait().unwrap();
    }
    assert!(observed_timeout.is_ok());
    assert!(matches!(early, Err(mpsc::TryRecvError::Empty)));
    assert_eq!(count_before_release, 1);
    assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
    let report = returned.recv().unwrap();
    assert_eq!(report.result, ComparisonResult::TimedOut);
    assert_eq!(report.exit_code(), 2);
}
