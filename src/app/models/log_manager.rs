use std::collections::VecDeque;

const REGULAR_LOG_CAP: usize = 1000;
const IMPORTANT_LOG_CAP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogRetention {
    Regular,
    Important,
    Sticky,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogSource {
    Application,
    ProcessMonitor,
}

/// Represents a single log entry with a message and a timestamp.
pub struct LogEntry {
    pub(crate) source: LogSource,
    pub sequence: u64,
    pub message: String,
    pub timestamp: std::time::SystemTime,
    pub retention: LogRetention,
}

impl LogEntry {
    /// Formats the log entry as a string: "[HH:MM:SS] :: message"
    pub fn format(&self) -> String {
        let duration = self
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        let secs = duration.as_secs();
        format!(
            "[{:02}:{:02}:{:02}] :: {}",
            (secs % 86400) / 3600,
            (secs % 3600) / 60,
            secs % 60,
            self.message
        )
    }
}

/// Manages application log entries with timestamps.
/// This structure is responsible for storing and formatting log messages
/// that can be displayed to the user for debugging and informational purposes.
#[derive(Default)]
pub struct LogManager {
    /// Chronological log entries with bounded retention for non-sticky classes.
    pub entries: VecDeque<LogEntry>,
    local_crash_context: Option<String>,
    sequence: u64,
    epoch: u64,
}

impl LogManager {
    fn push_entry(&mut self, message: String, retention: LogRetention) {
        self.push_source_entry(message, retention, LogSource::Application);
    }
    fn push_source_entry(&mut self, message: String, retention: LogRetention, source: LogSource) {
        if self.sequence == u64::MAX {
            self.clear();
            self.sequence = 0;
        }
        self.sequence += 1;
        let entry = LogEntry {
            source,
            sequence: self.sequence,
            message,
            timestamp: std::time::SystemTime::now(),
            retention,
        };

        #[cfg(debug_assertions)]
        println!("{}", entry.format());

        self.entries.push_back(entry);
        self.enforce_retention(retention);
    }

    fn enforce_retention(&mut self, retention: LogRetention) {
        let cap = match retention {
            LogRetention::Regular => Some(REGULAR_LOG_CAP),
            LogRetention::Important => Some(IMPORTANT_LOG_CAP),
            LogRetention::Sticky => None,
        };

        let Some(cap) = cap else {
            return;
        };

        while self
            .entries
            .iter()
            .filter(|entry| entry.retention == retention)
            .count()
            > cap
        {
            if let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.retention == retention)
            {
                self.entries.remove(index);
            } else {
                break;
            }
        }
    }

    fn has_sticky_message(&self, message: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.retention == LogRetention::Sticky && entry.message == message)
    }

    /// Adds a new regular log entry.
    pub fn add_entry(&mut self, message: String) {
        self.push_entry(message, LogRetention::Regular);
    }
    pub(crate) fn add_monitor_entry(&mut self, message: String, sticky: bool) {
        if sticky && self.has_sticky_message(&message) {
            return;
        }
        self.push_source_entry(
            message,
            if sticky {
                LogRetention::Sticky
            } else {
                LogRetention::Regular
            },
            LogSource::ProcessMonitor,
        );
    }
    pub(crate) fn monitor_entries(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|entry| entry.source == LogSource::ProcessMonitor)
            .take(100)
    }
    pub(crate) fn add_monitor_important_entry(&mut self, message: String) {
        self.push_source_entry(message, LogRetention::Important, LogSource::ProcessMonitor);
    }

    /// Adds a new important log entry.
    pub fn add_important_entry(&mut self, message: String) {
        self.push_entry(message, LogRetention::Important);
    }

    /// Adds a sticky log entry only once for the exact message.
    pub fn add_sticky_once(&mut self, message: String) {
        if self.has_sticky_message(&message) {
            return;
        }

        self.push_entry(message, LogRetention::Sticky);
    }

    pub(crate) fn add_important_sticky_once(&mut self, message: String) {
        self.add_important_entry(message.clone());
        self.add_sticky_once(message);
    }

    /// Replaces the local crash-report context shown independently in Activity.
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub(crate) fn replace_local_crash_context(&mut self, message: Option<String>) {
        self.local_crash_context = message;
    }

    pub(crate) fn local_crash_context(&self) -> Option<&str> {
        self.local_crash_context.as_deref()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
}

#[cfg(test)]
mod tests {
    use super::{LogManager, LogRetention, IMPORTANT_LOG_CAP, REGULAR_LOG_CAP};
    #[test]
    fn monitor_view_is_typed_bounded_and_does_not_match_unrelated_message_text() {
        let mut manager = LogManager::default();
        manager.add_entry("MONITOR: unrelated application text".into());
        for index in 0..105 {
            manager.add_monitor_entry(format!("Correction {index}"), false);
        }
        let rows: Vec<_> = manager.monitor_entries().collect();
        assert_eq!(rows.len(), 100);
        assert_eq!(rows[0].message, "Correction 104");
        assert_eq!(rows[99].message, "Correction 5");
        assert!(rows
            .iter()
            .all(|row| row.source == super::LogSource::ProcessMonitor));
    }

    #[test]
    fn log_sequences_survive_class_rotation_and_clear_changes_epoch() {
        let mut manager = LogManager::default();
        manager.add_important_entry("first".into());
        let first = manager.entries[0].sequence;
        for index in 0..(REGULAR_LOG_CAP + 5) {
            manager.add_entry(format!("{index}"));
        }
        let last = manager.entries.back().unwrap().sequence;
        assert!(last > first);
        assert_eq!(manager.entries[0].sequence, first);
        assert!(manager
            .entries
            .iter()
            .zip(manager.entries.iter().skip(1))
            .all(|(a, b)| a.sequence < b.sequence));
        let epoch = manager.epoch();
        manager.clear();
        manager.add_entry("after clear".into());
        assert_ne!(manager.epoch(), epoch);
        assert!(manager.entries[0].sequence > last);
    }

    #[test]
    fn formatting_preserves_time_and_multiline_unicode_message() {
        let entry = super::LogEntry {
            source: super::LogSource::Application,
            sequence: 1,
            message: "Ошибка\nsecond line".into(),
            timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(90_061),
            retention: LogRetention::Regular,
        };
        assert_eq!(entry.format(), "[01:01:01] :: Ошибка\nsecond line");
    }

    #[test]
    fn test_regular_retention_is_capped() {
        let mut manager = LogManager::default();
        for index in 0..(REGULAR_LOG_CAP + 5) {
            manager.add_entry(format!("regular-{index}"));
        }

        assert_eq!(
            manager
                .entries
                .iter()
                .filter(|entry| entry.retention == LogRetention::Regular)
                .count(),
            REGULAR_LOG_CAP
        );
        assert_eq!(
            manager.entries.front().map(|entry| entry.message.as_str()),
            Some("regular-5")
        );
    }

    #[test]
    fn test_important_retention_is_capped() {
        let mut manager = LogManager::default();
        for index in 0..(IMPORTANT_LOG_CAP + 3) {
            manager.add_important_entry(format!("important-{index}"));
        }

        assert_eq!(
            manager
                .entries
                .iter()
                .filter(|entry| entry.retention == LogRetention::Important)
                .count(),
            IMPORTANT_LOG_CAP
        );
        assert_eq!(
            manager.entries.front().map(|entry| entry.message.as_str()),
            Some("important-3")
        );
    }

    #[test]
    fn test_sticky_entries_survive_rotation_and_dedupe() {
        let mut manager = LogManager::default();
        manager.add_sticky_once("sticky".into());
        manager.add_sticky_once("sticky".into());

        for index in 0..(REGULAR_LOG_CAP + IMPORTANT_LOG_CAP + 50) {
            manager.add_entry(format!("regular-{index}"));
        }

        assert_eq!(
            manager
                .entries
                .iter()
                .filter(|entry| entry.retention == LogRetention::Sticky)
                .count(),
            1
        );
        assert!(manager
            .entries
            .iter()
            .any(|entry| entry.message == "sticky"));
    }

    #[test]
    fn test_clear_removes_all_classes() {
        let mut manager = LogManager::default();
        manager.add_entry("regular".into());
        manager.add_important_entry("important".into());
        manager.add_sticky_once("sticky".into());

        manager.clear();

        assert!(manager.entries.is_empty());
    }

    #[test]
    fn local_crash_context_replaces_the_previous_report_and_survives_clear() {
        let mut manager = LogManager::default();
        manager.add_entry("transient activity".into());
        manager.replace_local_crash_context(Some("previous crash one".into()));
        manager.replace_local_crash_context(Some("previous crash two".into()));

        manager.clear();

        assert!(manager.entries.is_empty());
        assert_eq!(manager.local_crash_context(), Some("previous crash two"));

        manager.replace_local_crash_context(None);
        assert!(manager.local_crash_context().is_none());
    }

    #[test]
    fn test_important_sticky_once_keeps_one_sticky_copy() {
        let mut manager = LogManager::default();

        manager.add_important_sticky_once("critical".into());
        manager.add_important_sticky_once("critical".into());

        assert_eq!(
            manager
                .entries
                .iter()
                .filter(|entry| {
                    entry.retention == LogRetention::Important && entry.message == "critical"
                })
                .count(),
            2
        );
        assert_eq!(
            manager
                .entries
                .iter()
                .filter(|entry| {
                    entry.retention == LogRetention::Sticky && entry.message == "critical"
                })
                .count(),
            1
        );
    }
}
