/// Represents a single log entry with a message and a timestamp.
pub struct LogEntry {
    pub message: String,
    pub timestamp: std::time::SystemTime,
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
    /// Vector of log entries
    pub entries: Vec<LogEntry>,
}

impl LogManager {
    /// Adds a new log entry.
    ///
    /// # Parameters
    ///
    /// * `message` - The log message to add
    pub fn add_entry(&mut self, message: String) {
        let entry = LogEntry {
            message,
            timestamp: std::time::SystemTime::now(),
        };

        #[cfg(debug_assertions)]
        println!("{}", entry.format());

        self.entries.push(entry);
    }

    /// Returns an iterator that yields formatted log strings.
    pub fn formatted_entries(&self) -> impl DoubleEndedIterator<Item = String> + '_ {
        self.entries.iter().map(|entry| entry.format())
    }
}
