//! A short, user-triggered activation session, separate from stale minimized input.
use os_api::WindowActivation;
use std::time::{Duration, Instant};

pub(super) struct RestoreFocus {
    deadline: Instant,
    next: Instant,
    remaining: u8,
}

impl RestoreFocus {
    pub fn new(now: Instant) -> Self {
        Self {
            deadline: now + Duration::from_millis(300),
            next: now,
            remaining: 2,
        }
    }

    /// None ends the session; Some schedules only the next bounded observation.
    pub fn poll(
        &mut self,
        now: Instant,
        activate: impl FnOnce() -> WindowActivation,
    ) -> Option<Duration> {
        if now >= self.deadline {
            return None;
        }
        if now < self.next {
            return Some(self.next - now);
        }
        let delay = match activate() {
            WindowActivation::Foreground | WindowActivation::Cancelled => return None,
            WindowActivation::NotReady => Duration::from_millis(25),
            WindowActivation::Denied => {
                self.remaining -= 1;
                if self.remaining == 0 {
                    return None;
                }
                Duration::from_millis(75)
            }
        };
        self.next = (now + delay).min(self.deadline);
        Some(self.next - now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_restore_attempts_activation_in_the_same_gui_turn() {
        let now = Instant::now();
        let mut focus = RestoreFocus::new(now);
        let mut attempted = false;
        assert_eq!(
            focus.poll(now, || {
                attempted = true;
                WindowActivation::Foreground
            }),
            None
        );
        assert!(
            attempted,
            "do not add a frame delay before activating an already-restored native window"
        );
    }

    #[test]
    fn waits_for_native_restore_and_retries_denial_only_once() {
        let now = Instant::now();
        let mut focus = RestoreFocus::new(now);
        assert!(focus.poll(now, || { WindowActivation::NotReady }).is_some());
        assert!(focus
            .poll(now + Duration::from_millis(25), || {
                WindowActivation::Denied
            })
            .is_some());
        assert!(focus
            .poll(now + Duration::from_millis(50), || panic!("retry too soon"))
            .is_some());
        assert_eq!(
            focus.poll(now + Duration::from_millis(100), || {
                WindowActivation::Denied
            }),
            None
        );
    }

    #[test]
    fn success_invalid_window_and_expiry_end_activation() {
        for outcome in [WindowActivation::Foreground, WindowActivation::Cancelled] {
            let now = Instant::now();
            let mut focus = RestoreFocus::new(now);
            assert_eq!(focus.poll(now, || outcome), None);
        }
        let now = Instant::now();
        let mut focus = RestoreFocus::new(now);
        assert_eq!(
            focus.poll(now + Duration::from_millis(300), || panic!(
                "expired request must not steal focus"
            )),
            None
        );
    }
}
