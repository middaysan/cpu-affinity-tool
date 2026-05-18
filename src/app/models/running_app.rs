#![allow(dead_code)]
use std::collections::HashMap;

use crate::app::models::AppRuntimeKey;
use crate::app::shared::ids::{GroupId, RuleId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppStatus {
    NotRunning,
    Running,
    SettingsMismatch,
}

/// Represents a single running application instance.
/// This structure tracks information about a running application,
/// including its process IDs, logical group/rule identities, and creation time.
pub struct RunningApp {
    /// List of process IDs associated with this application
    pub pids: Vec<u32>,
    /// Logical group identity for the tracked rule.
    pub group_id: GroupId,
    /// Logical rule identity for the tracked rule.
    pub rule_id: RuleId,
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
    pub apps: HashMap<AppRuntimeKey, RunningApp>,
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
    /// * `group_id` - The logical group identity
    /// * `rule_id` - The logical rule identity
    pub fn add_app(
        &mut self,
        app_key: &AppRuntimeKey,
        pid: u32,
        group_id: GroupId,
        rule_id: RuleId,
    ) {
        self.apps.insert(
            app_key.clone(),
            RunningApp {
                pids: vec![pid],
                group_id,
                rule_id,
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
    pub fn remove_app(&mut self, app_key: &AppRuntimeKey) {
        self.apps.remove(app_key);
    }
}
