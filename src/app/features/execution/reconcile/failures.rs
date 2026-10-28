use crate::app::shared::ids::RuleId;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const MAX_RETAINED_FAILURES: usize = 256;
const REPEAT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Failure {
    key: (RuleId, u32, u64),
    pub(super) message: String,
}
impl Failure {
    pub(super) fn new(rule: &RuleId, name: &str, pid: u32, token: u64, reason: &str) -> Self {
        Self {
            key: (rule.clone(), pid, token),
            message: format!(
                "Could not apply process settings for {} (PID {pid}): {}",
                bounded_text(name, 128),
                bounded_text(reason, 1024),
            ),
        }
    }
}

#[derive(Default)]
pub(super) struct FailureLog {
    previous: HashMap<(RuleId, u32, u64), (String, Instant)>,
    overflow_reported: Option<Instant>,
}
impl FailureLog {
    pub(super) fn report(&mut self, failures: Vec<Failure>, now: Instant) -> Vec<String> {
        // A successful iteration, Off or withdrawn membership ends the failure episode.
        let active: HashSet<_> = failures.iter().map(|failure| &failure.key).collect();
        self.previous.retain(|key, _| active.contains(key));
        let mut messages = Vec::new();
        let mut overflow = 0usize;
        for failure in failures {
            if let Some((message, reported)) = self.previous.get(&failure.key) {
                if message == &failure.message
                    && now.saturating_duration_since(*reported) < REPEAT_INTERVAL
                {
                    continue;
                }
            } else if self.previous.len() >= MAX_RETAINED_FAILURES {
                overflow += 1;
                continue;
            }
            self.previous
                .insert(failure.key, (failure.message.clone(), now));
            messages.push(failure.message);
        }
        if overflow == 0 {
            self.overflow_reported = None;
        } else if self
            .overflow_reported
            .is_none_or(|reported| now.saturating_duration_since(reported) >= REPEAT_INTERVAL)
        {
            self.overflow_reported = Some(now);
            messages.push(format!(
                "Could not apply process settings to {overflow} additional process instance(s); detailed monitor error limit reached. Other processes are still checked."
            ));
        }
        messages
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut text: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(token: u64, reason: &str) -> Failure {
        Failure::new(&RuleId("game-rule".into()), "Game", 42, token, reason)
    }

    #[test]
    fn s113_errors_repeat_at_most_once_a_minute_and_reset_on_recovery_or_pid_reuse() {
        let mut log = FailureLog::default();
        let now = Instant::now();
        assert_eq!(log.report(vec![failure(1, "Access denied")], now).len(), 1);
        assert!(log
            .report(
                vec![failure(1, "Access denied")],
                now + Duration::from_secs(3)
            )
            .is_empty());
        assert_eq!(
            log.report(vec![failure(1, "Access denied")], now + REPEAT_INTERVAL)
                .len(),
            1
        );
        assert_eq!(
            log.report(vec![failure(1, "Other error")], now + REPEAT_INTERVAL)
                .len(),
            1
        );
        assert_eq!(
            log.report(vec![failure(2, "Other error")], now + REPEAT_INTERVAL)
                .len(),
            1
        );
        assert!(log.report(vec![], now + REPEAT_INTERVAL).is_empty());
        assert_eq!(
            log.report(vec![failure(2, "Other error")], now + REPEAT_INTERVAL)
                .len(),
            1
        );
    }

    #[test]
    fn s113_error_bursts_keep_cache_text_and_overflow_reports_bounded() {
        let mut log = FailureLog::default();
        let now = Instant::now();
        let burst = || {
            (0..=MAX_RETAINED_FAILURES)
                .map(|token| failure(token as u64, "Denied"))
                .collect()
        };
        let first = log.report(burst(), now);
        assert_eq!(first.len(), MAX_RETAINED_FAILURES + 1);
        assert!(first.last().unwrap().contains("1 additional process"));
        assert_eq!(log.previous.len(), MAX_RETAINED_FAILURES);
        assert!(log.report(burst(), now + Duration::from_secs(3)).is_empty());
        let long = Failure::new(
            &RuleId("rule".into()),
            &"🎮".repeat(500),
            7,
            1,
            &"錯".repeat(5000),
        );
        assert!(long.message.contains("(PID 7)"));
        assert!(long.message.chars().count() < 128 + 1024 + 100);
    }
}
