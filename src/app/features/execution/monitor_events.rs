use crate::app::features::diagnostics::DiagnosticEvent;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;

/// A small, non-blocking boundary between monitor tasks and the GUI thread.
///
/// Monitor workers must never wait for an unresponsive GUI.  Runtime-state
/// notifications are deliberately coalesced because rendering only needs one
/// repaint to observe the latest shared state.  Text notifications are bounded
/// as well; the GUI reports a compact loss summary instead of allowing an
/// unbounded backlog to retain old process details.
pub(crate) const MONITOR_EVENT_QUEUE_CAPACITY: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MonitorDrainStatus {
    pub needs_repaint: bool,
    pub has_more_work: bool,
    pub dropped_monitor_messages: usize,
    pub dropped_warnings: usize,
}

#[derive(Clone)]
pub(crate) struct MonitorEventSender {
    tx: SyncSender<DiagnosticEvent>,
    queued: Arc<AtomicUsize>,
    repaint_pending: Arc<AtomicBool>,
    dropped_monitor_messages: Arc<AtomicUsize>,
    dropped_warnings: Arc<AtomicUsize>,
}

pub(crate) struct MonitorEventReceiver {
    rx: Receiver<DiagnosticEvent>,
    queued: Arc<AtomicUsize>,
    repaint_pending: Arc<AtomicBool>,
    dropped_monitor_messages: Arc<AtomicUsize>,
    dropped_warnings: Arc<AtomicUsize>,
}

pub(crate) fn monitor_event_channel() -> (MonitorEventSender, MonitorEventReceiver) {
    let (tx, rx) = mpsc::sync_channel(MONITOR_EVENT_QUEUE_CAPACITY);
    let queued = Arc::new(AtomicUsize::new(0));
    let repaint_pending = Arc::new(AtomicBool::new(false));
    let dropped_monitor_messages = Arc::new(AtomicUsize::new(0));
    let dropped_warnings = Arc::new(AtomicUsize::new(0));

    (
        MonitorEventSender {
            tx,
            queued: queued.clone(),
            repaint_pending: repaint_pending.clone(),
            dropped_monitor_messages: dropped_monitor_messages.clone(),
            dropped_warnings: dropped_warnings.clone(),
        },
        MonitorEventReceiver {
            rx,
            queued,
            repaint_pending,
            dropped_monitor_messages,
            dropped_warnings,
        },
    )
}

impl MonitorEventSender {
    /// Enqueues an event without blocking a monitor task.
    ///
    /// A full queue intentionally drops old-cycle text events. `RuntimeStateChanged`
    /// is coalesced into a repaint request, so the GUI still observes the latest
    /// runtime state even when no queue slot was available for that marker.
    pub(crate) fn try_send(&self, event: DiagnosticEvent) {
        if matches!(event, DiagnosticEvent::RuntimeStateChanged)
            && self.repaint_pending.swap(true, Ordering::AcqRel)
        {
            return;
        }

        match self.tx.try_send(event) {
            Ok(()) => {
                self.queued.fetch_add(1, Ordering::Release);
            }
            Err(TrySendError::Full(event)) => match event {
                DiagnosticEvent::RuntimeStateChanged => {}
                DiagnosticEvent::Monitor(_) => {
                    self.dropped_monitor_messages
                        .fetch_add(1, Ordering::Relaxed);
                    self.repaint_pending.store(true, Ordering::Release);
                }
                DiagnosticEvent::Warning(_) => {
                    self.dropped_warnings.fetch_add(1, Ordering::Relaxed);
                    self.repaint_pending.store(true, Ordering::Release);
                }
            },
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl MonitorEventReceiver {
    pub(crate) fn try_recv(&self) -> Result<DiagnosticEvent, TryRecvError> {
        let event = self.rx.try_recv()?;
        self.queued.fetch_sub(1, Ordering::AcqRel);
        if matches!(event, DiagnosticEvent::RuntimeStateChanged) {
            self.repaint_pending.store(false, Ordering::Release);
        }
        Ok(event)
    }

    /// Returns loss/repaint information after one bounded GUI drain.
    pub(crate) fn finish_drain(&self) -> MonitorDrainStatus {
        let dropped_monitor_messages = self.dropped_monitor_messages.swap(0, Ordering::AcqRel);
        let dropped_warnings = self.dropped_warnings.swap(0, Ordering::AcqRel);
        let needs_repaint = self.repaint_pending.swap(false, Ordering::AcqRel)
            || dropped_monitor_messages > 0
            || dropped_warnings > 0;
        let has_more_work = self.queued.load(Ordering::Acquire) > 0
            || self.repaint_pending.load(Ordering::Acquire)
            || self.dropped_monitor_messages.load(Ordering::Acquire) > 0
            || self.dropped_warnings.load(Ordering::Acquire) > 0;

        MonitorDrainStatus {
            needs_repaint,
            has_more_work,
            dropped_monitor_messages,
            dropped_warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{monitor_event_channel, MONITOR_EVENT_QUEUE_CAPACITY};
    use crate::app::shell::events::ShellEvent;

    #[test]
    fn runtime_state_notifications_are_coalesced() {
        let (tx, rx) = monitor_event_channel();

        tx.try_send(ShellEvent::RuntimeStateChanged);
        tx.try_send(ShellEvent::RuntimeStateChanged);

        assert_eq!(rx.try_recv().unwrap(), ShellEvent::RuntimeStateChanged);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn full_queue_drops_text_without_blocking_and_requests_repaint() {
        let (tx, rx) = monitor_event_channel();
        for index in 0..MONITOR_EVENT_QUEUE_CAPACITY {
            tx.try_send(ShellEvent::Monitor(format!("event-{index}")));
        }

        tx.try_send(ShellEvent::Monitor("dropped-monitor".to_string()));
        tx.try_send(ShellEvent::Warning("dropped-warning".to_string()));
        tx.try_send(ShellEvent::RuntimeStateChanged);

        for _ in 0..MONITOR_EVENT_QUEUE_CAPACITY {
            rx.try_recv().unwrap();
        }
        let status = rx.finish_drain();
        assert!(status.needs_repaint);
        assert_eq!(status.dropped_monitor_messages, 1);
        assert_eq!(status.dropped_warnings, 1);
    }

    #[test]
    fn finish_drain_reports_remaining_work_after_a_bounded_batch() {
        let (tx, rx) = monitor_event_channel();
        tx.try_send(ShellEvent::Monitor("one".to_string()));
        tx.try_send(ShellEvent::Monitor("two".to_string()));

        rx.try_recv().unwrap();
        assert!(rx.finish_drain().has_more_work);
    }
}
