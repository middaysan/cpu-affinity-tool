#![allow(dead_code)]
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppStatus {
    NotRunning,
    Running,
    SettingsMismatch,
}

/// Represents a single running application instance.
/// This structure tracks information about a running application,
/// including its process IDs, group and program indices, and creation time.
pub struct RunningApp {
    /// List of process IDs associated with this application
    pub pids: Vec<u32>,
    /// Index of the group this application belongs to
    pub group_index: usize,
    /// Index of the program within the group
    pub prog_index: usize,
    /// Time when the application was started
    pub created_at: std::time::SystemTime,
    /// Whether the CPU affinity and priority settings match the desired values
    pub settings_matched: bool,
}

/// Manages a collection of running applications.
/// This structure provides methods for adding and removing applications
/// from the collection, indexed by a unique key.
#[derive(Default)]
pub struct RunningApps {
    /// Map of application keys to RunningApp instances
    pub apps: HashMap<String, RunningApp>,
}

impl RunningApps {
    /// Adds a new running application to the collection.
    ///
    /// Creates a new RunningApp instance with the specified parameters
    /// and adds it to the collection, indexed by the provided key.
    ///
    /// # Parameters
    ///
    /// * `app_key` - A unique key to identify the application
    /// * `pid` - The process ID of the application
    /// * `group_index` - The index of the group the application belongs to
    /// * `prog_index` - The index of the program within the group
    pub fn add_app(&mut self, app_key: &str, pid: u32, group_index: usize, prog_index: usize) {
        self.apps.insert(
            app_key.to_string(),
            RunningApp {
                pids: vec![pid],
                group_index,
                prog_index,
                created_at: std::time::SystemTime::now(),
                settings_matched: true, // Default to true until checked by monitor
            },
        );
    }

    /// Removes an application from the collection.
    ///
    /// # Parameters
    ///
    /// * `app_key` - The key of the application to remove
    pub fn remove_app(&mut self, app_key: &str) {
        self.apps.remove(app_key);
    }
}
