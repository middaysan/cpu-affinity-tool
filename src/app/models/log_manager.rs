

pub struct LogManager {
    pub entries: Vec<String>,
}

impl LogManager {
    /// Add a new log entry with a timestamp.
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
