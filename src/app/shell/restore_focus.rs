//! A short, user-triggered activation session, separate from stale minimized input.
use os_api::WindowActivation;
use std::time::{Duration, Instant};

pub(super) struct RestoreFocus {
    deadline: Instant,
    next: Instant,
    input_tick: u32,
    remaining: u8,
}

impl RestoreFocus {
    pub fn new(now: Instant, input_tick: u32) -> Self {
        Self {
            deadline: now + Duration::from_millis(300),
            next: now + Duration::from_millis(16),
            input_tick,
            remaining: 2,
        }
    }

    /// None ends the session; Some schedules only the next bounded observation.
    pub fn poll(
        &mut self,
        now: Instant,
        activate: impl FnOnce(u32) -> WindowActivation,
    ) -> Option<Duration> {
        if now >= self.deadline {
            return None;
        }
        if now < self.next {
            return Some(self.next - now);
        }
        let delay = match activate(self.input_tick) {
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
    fn waits_for_native_restore_and_retries_denial_only_once() {
        let now = Instant::now();
        let mut focus = RestoreFocus::new(now, 42);
        assert_eq!(
            focus.poll(now, |_| panic!("viewport commands not applied yet")),
            Some(Duration::from_millis(16))
        );
        assert!(focus
            .poll(now + Duration::from_millis(16), |tick| {
                assert_eq!(tick, 42);
                WindowActivation::NotReady
            })
            .is_some());
        assert!(focus
            .poll(now + Duration::from_millis(41), |_| {
                WindowActivation::Denied
            })
            .is_some());
        assert!(focus
            .poll(now + Duration::from_millis(50), |_| panic!(
                "retry too soon"
            ))
            .is_some());
        assert_eq!(
            focus.poll(now + Duration::from_millis(116), |_| {
                WindowActivation::Denied
            }),
            None
        );
    }

    #[test]
    fn success_later_user_input_and_expiry_end_activation() {
        for outcome in [WindowActivation::Foreground, WindowActivation::Cancelled] {
            let now = Instant::now();
            let mut focus = RestoreFocus::new(now, 1);
            assert_eq!(
                focus.poll(now + Duration::from_millis(16), |_| outcome),
                None
            );
        }
        let now = Instant::now();
        let mut focus = RestoreFocus::new(now, 1);
        assert_eq!(
            focus.poll(now + Duration::from_millis(300), |_| panic!(
                "expired request must not steal focus"
            )),
            None
        );
    }
}
