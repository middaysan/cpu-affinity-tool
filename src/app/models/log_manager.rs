/// Manages application log entries with timestamps.
/// This structure is responsible for storing and formatting log messages
/// that can be displayed to the user for debugging and informational purposes.
pub struct LogManager {
    /// Vector of log entries, each formatted with a timestamp
    pub entries: Vec<String>,
}

impl LogManager {
    /// Adds a new log entry with a timestamp in the format [HH:MM:SS].
    ///
    /// Gets the current system time, formats it as a timestamp, and
    /// prepends it to the message before adding it to the entries list.
    ///
    /// # Parameters
    ///
    /// * `message` - The log message to add
    ///
    /// # Example
    ///
    /// ```
    /// let mut log_manager = LogManager { entries: vec![] };
    /// log_manager.add_entry("Application started".to_string());
    /// // Adds an entry like "[12:34:56] :: Application started"
    /// ```
    pub fn add_entry(&mut self, message: String) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let ts = format!(
            "[{:02}:{:02}:{:02}]",
            (now.as_secs() % 86400) / 3600,
            (now.as_secs() % 3600) / 60,
            now.as_secs() % 60
        );
        self.entries.push(format!("{} :: {}", ts, message));
    }
}
