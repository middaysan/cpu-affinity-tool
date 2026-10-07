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
fn start_control_reader(sender: std::sync::mpsc::SyncSender<logic::Control>) {
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        let _ = sender.send(logic::read_control(&mut input));
        // This reader owns no native authority and is never joined after a
        // control deadline; it may still be blocked reading a console byte.
    });
}
