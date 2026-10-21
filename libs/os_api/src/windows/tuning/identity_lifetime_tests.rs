use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CHILD_TEST: &str = "windows::tuning::identity::lifetime_tests::owned_lifetime_child";
const CHILD_ROLE: &str = "CPU_AFFINITY_TUNING_LIFETIME_CHILD";
const READY: &str = "CPU_AFFINITY_TUNING_LIFETIME_READY";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_PROTOCOL_LINES: usize = 64;
const MAX_PROTOCOL_LINE_BYTES: usize = 1024;

/// The test fixture can terminate only the child object it actually spawned.
/// There is no system enumeration, executable-name kill or recursive cleanup.
struct OwnedChild {
    child: Child,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    terminal: bool,
}
impl OwnedChild {
    fn ready() -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(CHILD_ROLE, "pipe_ready")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn our lifetime fixture");
        let output = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::sync_channel(MAX_PROTOCOL_LINES);
        let reader = thread::spawn(move || {
            let mut input = BufReader::new(output);
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
        let fixture = Self {
            child,
            lines,
            reader: Some(reader),
            terminal: false,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        for _ in 0..MAX_PROTOCOL_LINES {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = fixture
                .lines
                .recv_timeout(remaining)
                .expect("one overall child READY deadline");
            if line.trim_end().ends_with(READY) {
                return fixture;
            }
        }
        panic!("child output exceeded the bounded protocol");
    }
    fn terminate_and_wait(&mut self) {
        if !self.terminal {
            self.child.kill().expect("terminate only our fixture child");
            self.child.wait().expect("wait for our fixture child");
            self.terminal = true;
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.terminal {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        // Child death closes this pipe. Drain its bounded output before joining
        // so a sync sender cannot remain blocked during fixture cleanup.
        while self.lines.try_recv().is_ok() {}
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn process_wait_errors_and_unexpected_results_never_prove_exit() {
    assert_eq!(
        classify_process_wait(0, 0).unwrap(),
        TuningProcessLifetime::PositivelyExited
    );
    assert_eq!(
        classify_process_wait(258, 0).unwrap(),
        TuningProcessLifetime::VerifiedLive
    );
    assert!(matches!(
        classify_process_wait(u32::MAX, 5),
        Err(TuningNativeError::Denied { code: 5, .. })
    ));
    assert!(matches!(
        classify_process_wait(u32::MAX, 6),
        Err(TuningNativeError::Io { code: 6, .. })
    ));
    for unknown in [1, 128, 257, 259, 12345] {
        assert!(classify_process_wait(unknown, 0).is_err());
    }
}

#[test]
fn retained_guard_observes_our_verified_child_live_then_terminal() {
    let mut child = OwnedChild::ready();
    let expected = OS::tuning_process_identity(child.child.id()).unwrap();
    let mut guard = OS::tuning_open_process_lifetime(&expected)
        .expect("the exact process tuple must open a retained lifetime guard");
    assert_eq!(guard.poll().unwrap(), TuningProcessLifetime::VerifiedLive);
    let debug = format!("{guard:?}");
    assert!(!debug.contains(expected.image_path()));
    assert!(!debug.contains(expected.token().sid()));
    child.terminate_and_wait();
    assert_eq!(
        guard.poll().unwrap(),
        TuningProcessLifetime::PositivelyExited
    );
    assert_eq!(
        guard.poll().unwrap(),
        TuningProcessLifetime::PositivelyExited
    );
}

#[test]
fn lifetime_factory_rejects_every_stale_expected_tuple_field() {
    let actual = OS::tuning_process_identity(std::process::id()).unwrap();
    for field in [
        "pid",
        "creation",
        "image",
        "sid",
        "session",
        "authentication",
    ] {
        let mut stale = actual.clone();
        match field {
            "pid" => stale.pid = 0,
            "creation" => stale.creation_time = stale.creation_time.wrapping_add(1),
            "image" => stale.image_path.push_str(".different-target"),
            "sid" => stale.token.sid = "S-1-5-21-1-2-3-4999".to_owned(),
            "session" => stale.token.session_id = stale.token.session_id.wrapping_add(1),
            "authentication" => stale.token.authentication_id ^= 0x1_0000_0001,
            _ => unreachable!(),
        }
        assert!(
            OS::tuning_open_process_lifetime(&stale).is_err(),
            "stale {field} may not open native lifetime authority"
        );
    }
}

#[test]
fn failed_identity_observation_retains_unknown_error() {
    let expected = OS::tuning_process_identity(std::process::id()).unwrap();
    for error in [
        TuningNativeError::Denied {
            operation: "OpenProcessToken target",
            code: 5,
        },
        TuningNativeError::Io {
            operation: "GetProcessTimes identity",
            code: 6,
        },
        TuningNativeError::Unsupported("target process is not proven live"),
    ] {
        assert_eq!(
            require_expected_identity(&expected, Err(error.clone())),
            Err(error)
        );
    }
}

#[test]
fn live_poll_revalidates_every_field_on_the_retained_handle() {
    let actual = OS::tuning_process_identity(std::process::id()).unwrap();
    let mut guard = OS::tuning_open_process_lifetime(&actual)
        .expect("open current process for same-handle revalidation");
    for field in [
        "pid",
        "creation",
        "image",
        "sid",
        "session",
        "authentication",
    ] {
        guard.expected = actual.clone();
        match field {
            "pid" => guard.expected.pid = 0,
            "creation" => guard.expected.creation_time ^= 1,
            "image" => guard.expected.image_path.push_str(".different-target"),
            "sid" => guard.expected.token.sid = "S-1-5-21-1-2-3-4999".to_owned(),
            "session" => guard.expected.token.session_id ^= 1,
            "authentication" => guard.expected.token.authentication_id ^= 0x1_0000_0001,
            _ => unreachable!(),
        }
        assert!(guard.poll().is_err(), "changed {field} must stay Unknown");
    }
    guard.expected = actual;
    assert_eq!(guard.poll().unwrap(), TuningProcessLifetime::VerifiedLive);
}

#[test]
fn owned_lifetime_child() {
    if std::env::var(CHILD_ROLE).as_deref() != Ok("pipe_ready") {
        return;
    }
    println!("{READY}");
    std::io::stdout().flush().unwrap();
    let mut release = [0; 1];
    let _ = std::io::stdin().read_exact(&mut release);
}
