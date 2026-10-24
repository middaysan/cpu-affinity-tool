//! Functional work on a test-owned process is separate from real-role qualification.
#[path = "native_function_workload.rs"]
mod workload;

use super::{gaming, power, write_priority_on_handle, write_qos_on_handle};
use crate::{gaming::*, windows::OS};
use std::{
    io::{BufReader, Write},
    os::windows::process::CommandExt,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::System::Threading::{CREATE_NO_WINDOW, NORMAL_PRIORITY_CLASS};
use workload::{Progress, WorkReport, WorkRequest};

const CHILD_ENV: &str = "CPU_AFFINITY_BACKGROUND_FUNCTION_CHILD";
const CHILD_TEST: &str =
    "windows::background::native_tests::function_progress::native_owned_background_function_child";
const READY_PREFIX: &str = "cpu-affinity-ready-v1 ";
const DEADLINE: Duration = Duration::from_secs(30);
const PHASE_DEADLINE: Duration = Duration::from_secs(3);

enum ChildMessage {
    Ready(u32),
    Work(WorkReport),
}

/// Owns the exact spawned Windows process handle; it never kills by looked-up PID.
struct FunctionChild {
    child: Child,
    input: Option<ChildStdin>,
    replies: Receiver<Result<ChildMessage, String>>,
    reader: Option<JoinHandle<()>>,
    deadline: Instant,
}

impl FunctionChild {
    fn spawn() -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--ignored", "--nocapture", "--quiet"])
            .env(CHILD_ENV, "owned-v1")
            .creation_flags((CREATE_NO_WINDOW | NORMAL_PRIORITY_CLASS).0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (sender, replies) = mpsc::sync_channel(2);
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            for _ in 0..workload::MAX_REQUESTS + 16 {
                let message = match workload::read_bounded_line(&mut output) {
                    Ok(Some(line)) => {
                        if let Some(pid) = line.strip_prefix(READY_PREFIX) {
                            pid.trim_end_matches('\n')
                                .parse()
                                .map(ChildMessage::Ready)
                                .map_err(|_| "Invalid child readiness identity".to_string())
                        } else if line.starts_with(workload::PREFIX) {
                            workload::parse_report(&line)
                                .map(ChildMessage::Work)
                                .map_err(str::to_string)
                        } else {
                            continue;
                        }
                    }
                    Ok(None) => return,
                    Err(error) => Err(error.to_string()),
                };
                if sender.try_send(message).is_err() {
                    return;
                }
            }
            let _ = sender.try_send(Err("Child output exceeded its finite line budget".into()));
        });
        Self {
            child,
            input,
            replies,
            reader: Some(reader),
            deadline: Instant::now() + DEADLINE,
        }
    }

    fn reply(&self) -> ChildMessage {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "Owned functional probe exceeded 30 seconds"
        );
        self.replies
            .recv_timeout(remaining.min(PHASE_DEADLINE))
            .expect("Owned child made no bounded functional progress")
            .expect("Owned child returned an invalid functional response")
    }

    fn work(&mut self, progress: &mut Progress, phase: &str) {
        let request = WorkRequest {
            sequence: progress.sequence + 1,
            units: 500_000,
        };
        let input = self.input.as_mut().unwrap();
        writeln!(input, "work {} {}", request.sequence, request.units).unwrap();
        input.flush().unwrap();
        let ChildMessage::Work(report) = self.reply() else {
            panic!("Child readiness cannot count as functional progress");
        };
        progress
            .accept(request, report)
            .expect("Child work result does not prove progress");
        eprintln!(
            "{phase}: verified work sequence={} total={} checksum={}",
            report.sequence, report.total, report.checksum
        );
    }

    fn finish(&mut self) {
        writeln!(self.input.as_mut().unwrap(), "quit").unwrap();
        self.input.take();
        let deadline = (Instant::now() + PHASE_DEADLINE).min(self.deadline);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "Owned functional child failed: {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Owned child did not exit after Quit"
            );
            thread::sleep(Duration::from_millis(10));
        }
        while !self.reader.as_ref().unwrap().is_finished() {
            assert!(
                Instant::now() < deadline,
                "Owned child's bounded output did not close"
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.reader.take().unwrap().join().unwrap();
    }
}

impl Drop for FunctionChild {
    fn drop(&mut self) {
        self.input.take();
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if self
            .reader
            .as_ref()
            .is_some_and(|reader| reader.is_finished())
        {
            let _ = self.reader.take().unwrap().join();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeFields {
    priority: u32,
    qos: PowerThrottling,
}

fn read_fields(process: &ProcessIdentity) -> NativeFields {
    // Open a fresh query handle and verify the creation token/path independently
    // from the setter's retained handle and immediate readback.
    gaming::with_process(process, false, |handle| {
        let priority = unsafe { windows::Win32::System::Threading::GetPriorityClass(handle) };
        if priority == 0 {
            return Err(super::super::native(
                "Independent functional priority read",
                windows::core::Error::from_thread(),
            ));
        }
        Ok(NativeFields {
            priority,
            qos: power(handle)?,
        })
    })
    .expect("Independent native field readback must be available")
}

fn qos_owned(value: PowerThrottling) -> OwnedPolicy {
    OwnedPolicy {
        control: value.control_mask & EXECUTION_SPEED,
        state: value.state_mask & EXECUTION_SPEED,
    }
}

fn qos_fields(original: PowerThrottling, owned: OwnedPolicy) -> PowerThrottling {
    PowerThrottling {
        version: original.version,
        control_mask: (original.control_mask & !EXECUTION_SPEED) | owned.control,
        state_mask: (original.state_mask & !EXECUTION_SPEED) | owned.state,
    }
}

fn verify_work(
    child: &mut FunctionChild,
    process: &ProcessIdentity,
    expected: NativeFields,
    progress: &mut Progress,
    phase: &str,
) {
    assert_eq!(
        read_fields(process),
        expected,
        "Native fields before {phase}"
    );
    child.work(progress, phase);
    assert_eq!(
        read_fields(process),
        expected,
        "Native fields after {phase}"
    );
}

fn write_priority(process: &ProcessIdentity, expected: u32, desired: u32) {
    gaming::with_process(process, true, |handle| {
        write_priority_on_handle(handle, expected, desired)
    })
    .unwrap();
}

fn write_qos(process: &ProcessIdentity, expected: OwnedPolicy, desired: OwnedPolicy) {
    gaming::with_process(process, true, |handle| {
        write_qos_on_handle(handle, expected, desired)
    })
    .unwrap();
}

#[test]
#[ignore = "private finite arithmetic worker for the explicit test-owned functional probe"]
fn native_owned_background_function_child() {
    assert_eq!(std::env::var(CHILD_ENV).as_deref(), Ok("owned-v1"));
    // EOF closes the child when the parent dies; the watchdog also bounds an idle
    // but still-open pipe, including failed native test runners.
    thread::spawn(|| {
        thread::sleep(DEADLINE + Duration::from_secs(2));
        std::process::exit(124);
    });
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    writeln!(output, "\n{READY_PREFIX}{}", std::process::id()).unwrap();
    output.flush().unwrap();
    let mut progress = Progress::default();
    let mut checksum = 0u64;
    while let Some(line) = workload::read_bounded_line(&mut input).unwrap() {
        let Some(request) = workload::parse_request(&line).unwrap() else {
            return;
        };
        assert_eq!(request.sequence, progress.sequence + 1);
        let total = progress.total + u64::from(request.units);
        for unit in progress.total + 1..=total {
            let unit = std::hint::black_box(unit);
            checksum = checksum.wrapping_add(std::hint::black_box(unit.wrapping_mul(unit)));
        }
        progress.sequence = request.sequence;
        progress.total = total;
        writeln!(
            output,
            "{} {} {} {}",
            workload::PREFIX,
            request.sequence,
            total,
            std::hint::black_box(checksum)
        )
        .unwrap();
        output.flush().unwrap();
    }
}

#[test]
#[ignore = "explicit test-owned functional Priority/EcoQoS Apply/readback/work/exact Undo; no system roles"]
fn native_owned_background_function_progress() {
    let mut child = FunctionChild::spawn();
    let ChildMessage::Ready(pid) = child.reply() else {
        panic!("Missing exact-child readiness");
    };
    assert_eq!(pid, child.child.id());
    let identity = OS::tuning_process_identity(pid).unwrap();
    let process = ProcessIdentity {
        pid,
        creation_token: identity.creation_time(),
        image_path: identity.image_path().into(),
    };
    let original = read_fields(&process);
    assert_eq!(
        original.priority, NORMAL_PRIORITY_CLASS.0,
        "Owned fixture must start at explicitly requested Normal priority"
    );
    let original_owned = qos_owned(original.qos);
    let eco = OwnedPolicy {
        control: EXECUTION_SPEED,
        state: EXECUTION_SPEED,
    };
    let mut progress = Progress::default();
    eprintln!(
        "Owned child PID={} token={}; baseline={original:?}. This is mechanism/function evidence, not Windows-role admission.",
        process.pid, process.creation_token
    );
    verify_work(
        &mut child,
        &process,
        original,
        &mut progress,
        "original baseline",
    );

    let mut wrong = process.clone();
    wrong.creation_token ^= 1;
    let mut called = false;
    assert_eq!(
        gaming::with_process(&wrong, true, |handle| {
            called = true;
            write_priority_on_handle(handle, original.priority, 0x4000)
        }),
        Err(GamingError::Changed)
    );
    assert!(!called, "Wrong instance must be refused before the setter");
    assert_eq!(read_fields(&process), original);

    write_priority(&process, original.priority, 0x4000);
    verify_work(
        &mut child,
        &process,
        NativeFields {
            priority: 0x4000,
            ..original
        },
        &mut progress,
        "priority-only Apply",
    );
    write_priority(&process, 0x4000, original.priority);
    verify_work(
        &mut child,
        &process,
        original,
        &mut progress,
        "priority-only exact Undo",
    );

    for (label, baseline) in [
        (
            "SystemManaged",
            OwnedPolicy {
                control: 0,
                state: 0,
            },
        ),
        (
            "explicit Off",
            OwnedPolicy {
                control: EXECUTION_SPEED,
                state: 0,
            },
        ),
        ("explicit On", eco),
    ] {
        write_qos(&process, original_owned, baseline);
        let baseline_fields = NativeFields {
            qos: qos_fields(original.qos, baseline),
            ..original
        };
        verify_work(
            &mut child,
            &process,
            baseline_fields,
            &mut progress,
            &format!("EcoQoS {label} fixture baseline"),
        );
        write_qos(&process, baseline, eco);
        verify_work(
            &mut child,
            &process,
            NativeFields {
                qos: qos_fields(original.qos, eco),
                ..original
            },
            &mut progress,
            &format!("EcoQoS-only {label} Apply (changed={})", baseline != eco),
        );
        write_qos(&process, eco, baseline);
        verify_work(
            &mut child,
            &process,
            baseline_fields,
            &mut progress,
            &format!("EcoQoS-only {label} exact Undo"),
        );
        write_qos(&process, baseline, original_owned);
        assert_eq!(read_fields(&process), original);
    }

    write_priority(&process, original.priority, 0x4000);
    write_qos(&process, original_owned, eco);
    verify_work(
        &mut child,
        &process,
        NativeFields {
            priority: 0x4000,
            qos: qos_fields(original.qos, eco),
        },
        &mut progress,
        "combined Apply",
    );
    write_qos(&process, eco, original_owned);
    write_priority(&process, 0x4000, original.priority);
    verify_work(
        &mut child,
        &process,
        original,
        &mut progress,
        "combined exact Undo",
    );
    assert_eq!(progress.sequence, 14);
    assert_eq!(progress.total, 7_000_000);
    child.finish();
    eprintln!(
        "Owned child exited normally after 14 verified work batches; all original Priority/QoS fields restored. No CPU Sets, services or system roles changed; no performance/real-role qualification claim."
    );
}
