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

/// Runtime-only, bounded diagnostic evidence shown separately from ordinary
/// Activity entries. It is deliberately not part of the persisted state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsEventLogActivity {
    pub event_record_id: u64,
    pub timestamp_utc: String,
    pub exception_code: u32,
    pub faulting_module: String,
    pub stale: bool,
}

/// Manages application log entries with timestamps.
/// This structure is responsible for storing and formatting log messages
/// that can be displayed to the user for debugging and informational purposes.
#[derive(Default)]
pub struct LogManager {
    /// Chronological log entries with bounded retention for non-sticky classes.
    pub entries: VecDeque<LogEntry>,
    local_crash_context: Option<String>,
    windows_event_context: Option<WindowsEventLogActivity>,
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

    /// Replaces the local crash-report context shown independently in Activity.
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub(crate) fn replace_local_crash_context(&mut self, message: Option<String>) {
        self.local_crash_context = message;
    }

    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub(crate) fn replace_windows_event_context(
        &mut self,
        context: Option<WindowsEventLogActivity>,
    ) {
        self.windows_event_context = context;
    }

    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub(crate) fn mark_windows_event_context_stale(&mut self) {
        if let Some(context) = &mut self.windows_event_context {
            context.stale = true;
        }
    }

    pub(crate) fn local_crash_context(&self) -> Option<&str> {
        self.local_crash_context.as_deref()
    }

    pub(crate) fn windows_event_context(&self) -> Option<&WindowsEventLogActivity> {
        self.windows_event_context.as_ref()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns an iterator that yields formatted log strings.
    pub fn formatted_entries(&self) -> impl DoubleEndedIterator<Item = String> + '_ {
        self.entries.iter().map(|entry| entry.format())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LogManager, LogRetention, WindowsEventLogActivity, IMPORTANT_LOG_CAP, REGULAR_LOG_CAP,
    };

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
    fn retained_diagnostic_contexts_coexist_and_survive_clear() {
        let mut manager = LogManager::default();
        manager.add_entry("transient activity".into());
        manager.replace_local_crash_context(Some("local crash report".into()));
        manager.replace_windows_event_context(Some(WindowsEventLogActivity {
            event_record_id: 42,
            timestamp_utc: "2026-08-29T12:00:00.000Z".into(),
            exception_code: 0xc000_0005,
            faulting_module: "kernelbase.dll".into(),
            stale: false,
        }));

        manager.clear();

        assert!(manager.entries.is_empty());
        assert_eq!(manager.local_crash_context(), Some("local crash report"));
        assert_eq!(
            manager
                .windows_event_context()
                .map(|context| context.event_record_id),
            Some(42)
        );
        manager.mark_windows_event_context_stale();
        assert!(manager.windows_event_context().unwrap().stale);

        manager.replace_windows_event_context(None);
        assert!(manager.windows_event_context().is_none());
        assert_eq!(manager.local_crash_context(), Some("local crash report"));
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
