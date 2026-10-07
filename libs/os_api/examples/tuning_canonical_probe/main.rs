mod logic;
#[cfg(windows)]
mod native;

use logic::{Mode, Outcome, Report, Stage};

fn emit(report: &Report) -> bool {
    use std::io::Write;
    match report.line() {
        Ok(line) => {
            let mut output = std::io::stdout().lock();
            writeln!(output, "{line}").is_ok() && output.flush().is_ok()
        }
        Err(()) => false,
    }
}
fn main() {
    // This isolated helper reports panic only as a fixed WorkerPanicked code;
    // native error/panic strings must not disclose paths or identity payloads.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .take(2)
        .map(|arg| arg.into_string())
        .collect();
    #[cfg(windows)]
    if args
        .as_ref()
        .is_ok_and(|args| args == &[logic::PRIVATE_SAMPLE_ARGUMENT])
    {
        std::process::exit(private_sample_main());
    }
    let mode = args
        .as_ref()
        .ok()
        .and_then(|args| logic::parse_mode(args).ok());
    let Some(mode) = mode else {
        let mut report = Report::new(
            Mode::Preflight,
            Stage::Controller,
            Outcome::InvalidArguments,
        );
        report.worker_terminal = true;
        let _ = emit(&report);
        std::process::exit(2);
    };
    #[cfg(windows)]
    std::process::exit(windows_main(mode));
    #[cfg(not(windows))]
    {
        let mut report = Report::new(mode, Stage::Controller, Outcome::UnsupportedPlatform);
        report.worker_terminal = true;
        let _ = emit(&report);
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn windows_main(mode: Mode) -> i32 {
    if mode == Mode::LabContinuityCompare {
        return comparison_main();
    }
    use std::{sync::mpsc::RecvTimeoutError, time::Instant};
    let worker = logic::Worker::start(mode, || native::NativeBackend);
    let mut deadline = Instant::now() + logic::OPEN_DEADLINE;
    let mut timeout_reported = false;
    let mut input_started = false;
    let mut complete_output = true;
    loop {
        let remaining = if timeout_reported {
            std::time::Duration::from_secs(1)
        } else {
            deadline.saturating_duration_since(Instant::now())
        };
        match worker.reports.recv_timeout(remaining) {
            Ok(report) => {
                complete_output &= emit(&report);
                if report.worker_terminal {
                    let joined = worker.join();
                    return logic::terminal_exit_code(
                        &report,
                        joined,
                        timeout_reported,
                        complete_output,
                    );
                }
                if report.outcome == Outcome::Ready && !input_started {
                    input_started = true;
                    start_control_reader(worker.control.clone());
                    deadline = Instant::now() + logic::HOLD_DEADLINE + logic::OPEN_DEADLINE;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !timeout_reported {
                    timeout_reported = true;
                    complete_output &=
                        emit(&Report::new(mode, Stage::Controller, Outcome::Timeout));
                }
                // This process remains alive with the same outstanding worker.
                // No replacement, implicit main exit or claimed lease release.
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _joined = worker.join();
                let mut report = Report::new(mode, Stage::Closed, Outcome::WorkerPanicked);
                report.worker_terminal = true;
                let _ = emit(&report);
                return 2;
            }
        }
    }
}

#[cfg(windows)]
fn comparison_main() -> i32 {
    use std::{
        io::Write,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::Instant,
    };
    fn emit_comparison(report: &logic::ComparisonReport) -> bool {
        let Ok(line) = report.line() else {
            return false;
        };
        let mut output = std::io::stdout().lock();
        writeln!(output, "{line}").is_ok() && output.flush().is_ok()
    }
    if !cfg!(all(target_arch = "x86_64", target_pointer_width = "64")) {
        let _ = emit_comparison(&logic::ComparisonReport::failed(
            logic::ComparisonResult::Failed,
        ));
        return 2;
    }
    let Ok(executable) = std::env::current_exe() else {
        return 2;
    };
    let mut complete_output = true;
    let report = logic::run_comparison(
        || {
            let mut child = Command::new(&executable)
                .arg(logic::PRIVATE_SAMPLE_ARGUMENT)
                .creation_flags(0x0800_0000)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            // Closing this owned input completes the one exact handshake. Even a
            // failed write hands the actual Child to the retaining collector.
            if let Some(mut input) = child.stdin.take() {
                let _ = input.write_all(logic::SAMPLE_HANDSHAKE);
            }
            Ok(child)
        },
        Instant::now() + logic::OPEN_DEADLINE,
        || {
            complete_output &= emit_comparison(&logic::ComparisonReport::failed(
                logic::ComparisonResult::TimedOut,
            ));
        },
    );
    complete_output &= emit_comparison(&report);
    if complete_output {
        report.exit_code()
    } else {
        2
    }
}

#[cfg(windows)]
fn private_sample_main() -> i32 {
    use std::os::windows::io::AsRawHandle;
    use std::{io::Write, sync::mpsc, time::Duration};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
    };
    let pipes = unsafe {
        [
            std::io::stdin().as_raw_handle(),
            std::io::stdout().as_raw_handle(),
        ]
        .into_iter()
        .all(|handle| GetFileType(HANDLE(handle)) == FILE_TYPE_PIPE)
    };
    if !logic::sample_handshake(&mut std::io::stdin().lock(), pipes) {
        return 2;
    }
    let (sender, result) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let _ = sender.send(native::capture_private_sample());
    });
    let mut timed_out = false;
    let sample = loop {
        match result.recv_timeout(if timed_out {
            Duration::from_secs(1)
        } else {
            logic::OPEN_DEADLINE
        }) {
            Ok(sample) => break sample,
            Err(mpsc::RecvTimeoutError::Timeout) => timed_out = true,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(()),
        }
    };
    let joined = worker.join().is_ok();
    if timed_out || !joined {
        return 2;
    }
    let Ok(sample) = sample else {
        return 2;
    };
    let Ok(frame) = logic::encode_sample(&sample) else {
        return 2;
    };
    // This is the validated private inherited pipe, never emit()/public JSON.
    let mut output = std::io::stdout().lock();
    if output.write_all(&frame).is_ok() && output.flush().is_ok() {
        0
    } else {
        2
    }
}

#[cfg(windows)]
fn start_control_reader(sender: std::sync::mpsc::SyncSender<logic::Control>) {
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        let _ = sender.send(logic::read_control(&mut input));
        // This reader owns no native authority and is never joined after a
        // control deadline; it may still be blocked reading a console byte.
    });
}
