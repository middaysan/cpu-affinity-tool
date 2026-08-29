use os_api::WindowsApplicationFailure;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RETRY_DELAY: Duration = Duration::from_secs(10);
const WORKER_SLOW_AFTER: Duration = Duration::from_secs(2);

type ScanResult = Result<Option<WindowsApplicationFailure>, String>;
type ScanFn = dyn Fn() -> ScanResult + Send + Sync + 'static;

/// Sanitized Event Log evidence owned by the diagnostics manager. It is not a
/// chronological Activity entry and is never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsEventLogRecord {
    pub event_record_id: u64,
    pub timestamp_utc: String,
    pub exception_code: u32,
    pub faulting_module: String,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsEventLogState {
    Disabled,
    Idle,
    Loading {
        last_complete: Option<WindowsEventLogRecord>,
    },
    Ready {
        latest: Option<WindowsEventLogRecord>,
    },
    Incomplete {
        last_complete: Option<WindowsEventLogRecord>,
        reason: String,
    },
}

impl WindowsEventLogState {
    pub fn latest_complete(&self) -> Option<&WindowsEventLogRecord> {
        match self {
            Self::Ready { latest } => latest.as_ref(),
            Self::Loading { last_complete } | Self::Incomplete { last_complete, .. } => {
                last_complete.as_ref()
            }
            Self::Disabled | Self::Idle => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsEventLogPoll {
    Unchanged,
    Completed(ScanResult),
    MarkedIncomplete,
}

/// A one-shot, explicitly enabled Event Log reader. It intentionally has no
/// focus-triggered refresh path and never starts work from its constructor.
pub struct WindowsEventLogManager {
    enabled: bool,
    initial_scan_started: bool,
    retry_used: bool,
    retry_due: Option<Instant>,
    generation: u64,
    state: WindowsEventLogState,
    worker: Option<Receiver<(u64, ScanResult)>>,
    worker_started: Option<Instant>,
    rearm_after_worker: bool,
    scan: Arc<ScanFn>,
}

impl WindowsEventLogManager {
    pub fn new_idle() -> Self {
        Self::new_with_scan(Arc::new(|| {
            crate::app::adapters::os::find_latest_windows_application_failure()
        }))
    }

    fn new_with_scan(scan: Arc<ScanFn>) -> Self {
        Self {
            enabled: false,
            initial_scan_started: false,
            retry_used: false,
            retry_due: None,
            generation: 0,
            state: WindowsEventLogState::Disabled,
            worker: None,
            worker_started: None,
            rearm_after_worker: false,
            scan,
        }
    }

    #[cfg(test)]
    fn new_with_test_scan(scan: impl Fn() -> ScanResult + Send + Sync + 'static) -> Self {
        Self::new_with_scan(Arc::new(scan))
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> &WindowsEventLogState {
        &self.state
    }

    /// Returns the manager-owned typed UI snapshot. Callers must not copy it
    /// into the general Activity log because Event Log evidence has separate
    /// retention and consent semantics.
    pub(crate) fn ui_snapshot(&self) -> WindowsEventLogState {
        self.state.clone()
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn start_initial_scan(&mut self) -> bool {
        if !self.enabled || self.initial_scan_started {
            return false;
        }
        self.initial_scan_started = true;
        self.start_worker()
    }

    /// Enables diagnostics without starting a lookup. The shell gate must
    /// call `start_initial_scan` later, after the first rendered frame.
    pub fn enable(&mut self) -> bool {
        if self.enabled {
            return false;
        }

        self.enabled = true;
        self.initial_scan_started = false;
        self.retry_used = false;
        self.retry_due = None;
        self.generation = self.generation.wrapping_add(1);
        if self.worker.is_some() {
            // The old worker cannot be cancelled. Keep its slot until poll
            // observes completion, then rearm exactly once from that path.
            self.rearm_after_worker = true;
        } else {
            self.state = WindowsEventLogState::Idle;
            self.rearm_after_worker = false;
        }
        true
    }

    pub fn disable(&mut self) {
        if !self.enabled {
            return;
        }
        self.enabled = false;
        self.generation = self.generation.wrapping_add(1);
        self.retry_due = None;
        self.rearm_after_worker = false;
        self.state = WindowsEventLogState::Disabled;
    }

    pub fn poll(&mut self) -> WindowsEventLogPoll {
        let Some(worker) = self.worker.as_ref() else {
            self.start_retry_if_due();
            return WindowsEventLogPoll::Unchanged;
        };

        match worker.try_recv() {
            Ok((generation, result)) => {
                self.worker = None;
                self.worker_started = None;
                if !self.enabled || generation != self.generation {
                    if self.enabled && self.rearm_after_worker {
                        self.rearm_after_worker = false;
                        self.initial_scan_started = true;
                        let _ = self.start_worker();
                    }
                    return WindowsEventLogPoll::Unchanged;
                }

                self.apply_completed_result(&result);
                self.start_retry_if_due();
                WindowsEventLogPoll::Completed(result)
            }
            Err(TryRecvError::Disconnected) => {
                self.worker = None;
                self.worker_started = None;
                if self.enabled && self.rearm_after_worker {
                    self.rearm_after_worker = false;
                    self.initial_scan_started = true;
                    let _ = self.start_worker();
                    return WindowsEventLogPoll::Unchanged;
                }
                if self.enabled {
                    let last_complete = self.state.latest_complete().cloned();
                    self.state = WindowsEventLogState::Incomplete {
                        last_complete,
                        reason: "the Windows Event Log worker stopped unexpectedly".to_string(),
                    };
                    WindowsEventLogPoll::MarkedIncomplete
                } else {
                    WindowsEventLogPoll::Unchanged
                }
            }
            Err(TryRecvError::Empty) => {
                if self.enabled
                    && self
                        .worker_started
                        .is_some_and(|started| started.elapsed() >= WORKER_SLOW_AFTER)
                    && !matches!(self.state, WindowsEventLogState::Incomplete { .. })
                {
                    let last_complete = self.state.latest_complete().cloned();
                    self.state = WindowsEventLogState::Incomplete {
                        last_complete,
                        reason: "the Windows Event Log lookup is taking longer than expected"
                            .to_string(),
                    };
                    return WindowsEventLogPoll::MarkedIncomplete;
                }
                WindowsEventLogPoll::Unchanged
            }
        }
    }

    pub fn worker_poll_interval(&self) -> Option<Duration> {
        if self.worker.is_some() {
            return Some(
                if self
                    .worker_started
                    .is_none_or(|started| started.elapsed() < WORKER_SLOW_AFTER)
                {
                    Duration::from_millis(100)
                } else {
                    Duration::from_secs(2)
                },
            );
        }
        self.retry_due
            .map(|due| due.saturating_duration_since(Instant::now()))
    }

    #[cfg(test)]
    pub(crate) fn worker_is_active(&self) -> bool {
        self.worker.is_some()
    }

    #[cfg(test)]
    fn force_retry_due(&mut self) {
        self.retry_due = Some(Instant::now());
    }

    fn apply_completed_result(&mut self, result: &ScanResult) {
        match result {
            Ok(latest) => {
                self.state = WindowsEventLogState::Ready {
                    latest: latest.as_ref().map(|record| WindowsEventLogRecord {
                        event_record_id: record.event_record_id,
                        timestamp_utc: record.timestamp_utc.clone(),
                        exception_code: record.exception_code,
                        faulting_module: record.faulting_module.clone(),
                        stale: false,
                    }),
                };
                if latest.is_none() && !self.retry_used {
                    self.retry_used = true;
                    self.retry_due = Some(Instant::now() + RETRY_DELAY);
                }
            }
            Err(reason) => {
                let last_complete = self.state.latest_complete().cloned();
                self.state = WindowsEventLogState::Incomplete {
                    last_complete,
                    reason: reason.clone(),
                };
            }
        }
    }

    fn start_retry_if_due(&mut self) {
        if self.enabled
            && self.worker.is_none()
            && self.retry_due.is_some_and(|due| due <= Instant::now())
        {
            self.retry_due = None;
            let _ = self.start_worker();
        }
    }

    fn start_worker(&mut self) -> bool {
        if !self.enabled || self.worker.is_some() {
            return false;
        }

        let last_complete = self.state.latest_complete().cloned();
        self.state = WindowsEventLogState::Loading { last_complete };
        let (sender, receiver) = mpsc::channel();
        let scan = Arc::clone(&self.scan);
        let generation = self.generation;
        let spawn = std::thread::Builder::new()
            .name("windows-event-log-scan".to_string())
            .spawn(move || {
                let _ = sender.send((generation, scan()));
            });

        match spawn {
            Ok(_) => {
                self.worker = Some(receiver);
                self.worker_started = Some(Instant::now());
                true
            }
            Err(error) => {
                self.state = WindowsEventLogState::Incomplete {
                    last_complete: self.state.latest_complete().cloned(),
                    reason: format!("failed to start the Windows Event Log worker: {error}"),
                };
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WindowsEventLogManager, WindowsEventLogPoll, WindowsEventLogState, WORKER_SLOW_AFTER,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn poll_until_complete(manager: &mut WindowsEventLogManager) -> WindowsEventLogPoll {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let update = manager.poll();
            if !matches!(update, WindowsEventLogPoll::Unchanged) {
                return update;
            }
            assert!(Instant::now() < deadline, "worker did not complete");
            std::thread::yield_now();
        }
    }

    #[test]
    fn manager_does_not_start_a_lookup_before_the_shell_gate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let call_counter = Arc::clone(&calls);
        let manager = WindowsEventLogManager::new_with_test_scan(move || {
            call_counter.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        });

        assert!(!manager.worker_is_active());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn empty_initial_result_schedules_exactly_one_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let call_counter = Arc::clone(&calls);
        let mut manager = WindowsEventLogManager::new_with_test_scan(move || {
            call_counter.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        });

        assert!(manager.enable());
        assert!(manager.start_initial_scan());
        assert!(matches!(
            poll_until_complete(&mut manager),
            WindowsEventLogPoll::Completed(Ok(None))
        ));
        manager.force_retry_due();
        assert!(matches!(manager.poll(), WindowsEventLogPoll::Unchanged));
        assert!(manager.worker_is_active());
        assert!(matches!(
            poll_until_complete(&mut manager),
            WindowsEventLogPoll::Completed(Ok(None))
        ));

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(!manager.worker_is_active());
        assert!(manager.retry_due.is_none());
    }

    #[test]
    fn errors_do_not_schedule_a_retry() {
        let mut manager =
            WindowsEventLogManager::new_with_test_scan(|| Err("access denied".into()));

        assert!(manager.enable());
        assert!(manager.start_initial_scan());
        assert!(matches!(
            poll_until_complete(&mut manager),
            WindowsEventLogPoll::Completed(Err(_))
        ));

        assert!(!manager.worker_is_active());
        assert!(manager.retry_due.is_none());
        assert!(matches!(
            manager.state(),
            WindowsEventLogState::Incomplete { .. }
        ));
        assert!(manager.state().latest_complete().is_none());
    }

    #[test]
    fn disable_drops_a_completed_worker_result() {
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let release_receiver = Arc::new(std::sync::Mutex::new(release_receiver));
        let worker_receiver = Arc::clone(&release_receiver);
        let mut manager = WindowsEventLogManager::new_with_test_scan(move || {
            entered_sender.send(()).unwrap();
            worker_receiver.lock().unwrap().recv().unwrap();
            Ok(None)
        });

        manager.enable();
        assert!(manager.start_initial_scan());
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        manager.disable();
        release_sender.send(()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while manager.worker_is_active() && Instant::now() < deadline {
            let _ = manager.poll();
            std::thread::yield_now();
        }
        assert!(!manager.worker_is_active());
        assert_eq!(manager.state(), &WindowsEventLogState::Disabled);
    }

    #[test]
    fn disable_keeps_the_snapshot_disabled_while_a_slow_worker_drains() {
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let release_receiver = Arc::new(std::sync::Mutex::new(release_receiver));
        let worker_receiver = Arc::clone(&release_receiver);
        let mut manager = WindowsEventLogManager::new_with_test_scan(move || {
            entered_sender.send(()).unwrap();
            worker_receiver.lock().unwrap().recv().unwrap();
            Ok(None)
        });

        manager.enable();
        assert!(manager.start_initial_scan());
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        manager.disable();
        manager.worker_started = Some(Instant::now() - WORKER_SLOW_AFTER);

        assert!(matches!(manager.poll(), WindowsEventLogPoll::Unchanged));
        assert_eq!(manager.state(), &WindowsEventLogState::Disabled);

        release_sender.send(()).unwrap();
    }

    #[test]
    fn reenable_while_old_worker_is_in_flight_rearms_once_after_release() {
        let calls = Arc::new(AtomicUsize::new(0));
        let call_counter = Arc::clone(&calls);
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let release_receiver = Arc::new(std::sync::Mutex::new(release_receiver));
        let worker_receiver = Arc::clone(&release_receiver);
        let mut manager = WindowsEventLogManager::new_with_test_scan(move || {
            let call = call_counter.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 1 {
                entered_sender.send(()).unwrap();
                worker_receiver.lock().unwrap().recv().unwrap();
            }
            Ok(None)
        });

        manager.enable();
        assert!(manager.start_initial_scan());
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        manager.disable();
        assert!(manager.enable());
        assert!(!manager.start_initial_scan());
        release_sender.send(()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            let _ = manager.poll();
            std::thread::yield_now();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(manager.worker_is_active());
        assert!(matches!(
            poll_until_complete(&mut manager),
            WindowsEventLogPoll::Completed(Ok(None))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
