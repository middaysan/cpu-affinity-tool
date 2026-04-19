use std::collections::VecDeque;

const REGULAR_LOG_CAP: usize = 1000;
const IMPORTANT_LOG_CAP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogRetention {
    Regular,
    Important,
    Sticky,
}

/// Represents a single log entry with a message and a timestamp.
pub struct LogEntry {
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
        let ts = format!(
            "[{:02}:{:02}:{:02}]",
            (secs % 86400) / 3600, // hours
            (secs % 3600) / 60,    // minutes
            secs % 60              // seconds
        );

        format!("{ts} :: {}", self.message)
    }
}

/// Manages application log entries with timestamps.
/// This structure is responsible for storing and formatting log messages
/// that can be displayed to the user for debugging and informational purposes.
#[derive(Default)]
pub struct LogManager {
    /// Chronological log entries with bounded retention for non-sticky classes.
    pub entries: VecDeque<LogEntry>,
}

impl LogManager {
    fn push_entry(&mut self, message: String, retention: LogRetention) {
        let entry = LogEntry {
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

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns an iterator that yields formatted log strings.
    pub fn formatted_entries(&self) -> impl DoubleEndedIterator<Item = String> + '_ {
        self.entries.iter().map(|entry| entry.format())
    }
}

#[cfg(test)]
mod tests {
    use super::{LogManager, LogRetention, IMPORTANT_LOG_CAP, REGULAR_LOG_CAP};

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
