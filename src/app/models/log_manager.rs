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
    /// prepends it to the message before adding it to the entry list.
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
        // Get current time since UNIX epoch
        let duration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        // Format time as [HH:MM:SS] using simple time calculations
        let secs = duration.as_secs();
        let ts = format!(
            "[{:02}:{:02}:{:02}]",
            (secs % 86400) / 3600, // hours
            (secs % 3600) / 60,    // minutes
            secs % 60              // seconds
        );

        self.entries.push(format!("{ts} :: {message}"));
    }
}
