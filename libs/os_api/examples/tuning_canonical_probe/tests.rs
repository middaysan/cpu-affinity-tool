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
