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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::AppToRun;
    use os_api::PriorityClass;
    use std::path::PathBuf;

    fn key(name: &str) -> AppRuntimeKey {
        AppToRun::new_path(
            PathBuf::from(format!(r"C:\{name}.lnk")),
            Vec::new(),
            PathBuf::from(format!(r"C:\{name}.exe")),
            PriorityClass::Normal,
            false,
        )
        .get_key()
    }

    #[test]
    fn test_add_app_records_pid_logical_ids_and_default_status() {
        let mut apps = RunningApps::default();
        let app_key = key("Sample");
        let group_id = GroupId("group-a".to_string());
        let rule_id = RuleId("rule-a".to_string());

        apps.add_app(&app_key, 42, group_id.clone(), rule_id.clone());

        let app = apps.apps.get(&app_key).unwrap();
        assert_eq!(app.pids, vec![42]);
        assert_eq!(app.group_id, group_id);
        assert_eq!(app.rule_id, rule_id);
        assert!(app.settings_matched);
    }

    #[test]
    fn test_add_app_replaces_existing_entry_for_same_runtime_key() {
        let mut apps = RunningApps::default();
        let app_key = key("Sample");

        apps.add_app(
            &app_key,
            42,
            GroupId("group-a".to_string()),
            RuleId("rule-a".to_string()),
        );
        apps.add_app(
            &app_key,
            77,
            GroupId("group-b".to_string()),
            RuleId("rule-b".to_string()),
        );

        let app = apps.apps.get(&app_key).unwrap();
        assert_eq!(app.pids, vec![77]);
        assert_eq!(app.group_id, GroupId("group-b".to_string()));
        assert_eq!(app.rule_id, RuleId("rule-b".to_string()));
        assert_eq!(apps.apps.len(), 1);
    }

    #[test]
    fn test_remove_app_is_noop_for_missing_key() {
        let mut apps = RunningApps::default();
        let existing_key = key("Existing");
        let missing_key = key("Missing");
        apps.add_app(
            &existing_key,
            42,
            GroupId("group-a".to_string()),
            RuleId("rule-a".to_string()),
        );

        apps.remove_app(&missing_key);

        assert!(apps.apps.contains_key(&existing_key));
        assert!(!apps.apps.contains_key(&missing_key));
        assert_eq!(apps.apps.len(), 1);
    }
}
