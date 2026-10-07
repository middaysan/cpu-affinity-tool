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
struct Fake {
    evidence: Arc<Mutex<Evidence>>,
    token: TokenContext,
    failure: Option<Failure>,
    token_failure: Option<Failure>,
    validate_failure: Option<Failure>,
    read_failure: Option<Failure>,
    canonical: bool,
    presence: Presence,
}
struct FakeStore {
    evidence: Arc<Mutex<Evidence>>,
    canonical: bool,
    presence: Presence,
    validate_failure: Option<Failure>,
    read_failure: Option<Failure>,
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
