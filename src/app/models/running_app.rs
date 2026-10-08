#![allow(dead_code)]
use std::collections::HashMap;

use crate::app::models::AppRuntimeKey;
use crate::app::shared::ids::{GroupId, RuleId};

/// OS-provided token that distinguishes successive process instances sharing a PID.
pub type ProcessInstanceToken = u64;

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
    /// Creation tokens captured with `pids`; absent tokens are not monitor-retainable.
    pub pid_instance_tokens: HashMap<u32, ProcessInstanceToken>,
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
pub struct RunningApps {
    /// Map of application keys to RunningApp instances
    pub apps: HashMap<AppRuntimeKey, RunningApp>,
    /// Runtime-only two-field handoff shared with the registered scheduling modules.
    pub(crate) scheduling: std::sync::Arc<
        crate::app::features::tuning::modules::managed::scheduling::SchedulingHandoff,
    >,
    membership_generation: Option<u64>,
    editing_membership: bool,
}
impl Default for RunningApps {
    fn default() -> Self {
        Self {
            apps: HashMap::new(),
            scheduling: Default::default(),
            membership_generation: Some(0),
            editing_membership: false,
        }
    }
}

impl RunningApps {
    pub(crate) fn membership_generation(&self) -> Option<u64> {
        self.membership_generation
    }
    pub(crate) fn mutate_membership<R>(&mut self, edit: impl FnOnce(&mut Self) -> R) -> R {
        if self.editing_membership {
            return edit(self);
        }
        let signature = |apps: &Self| {
            apps.apps
                .iter()
                .map(|(key, app)| {
                    let mut pids = app.pids.clone();
                    pids.sort_unstable();
                    let mut tokens = app
                        .pid_instance_tokens
                        .iter()
                        .map(|(pid, token)| (*pid, *token))
                        .collect::<Vec<_>>();
                    tokens.sort_unstable();
                    (
                        key.clone(),
                        (app.group_id.clone(), app.rule_id.clone(), pids, tokens),
                    )
                })
                .collect::<HashMap<_, _>>()
        };
        let before = signature(self);
        // A panic leaves uncertainty sticky; it cannot publish a stale generation.
        let previous = self.membership_generation.take();
        self.editing_membership = true;
        let result = edit(self);
        self.editing_membership = false;
        self.membership_generation = if before != signature(self) {
            previous.and_then(|generation| generation.checked_add(1))
        } else {
            previous
        };
        result
    }
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
        self.mutate_membership(|apps| {
            apps.apps.insert(
                app_key.clone(),
                RunningApp {
                    pids: vec![pid],
                    pid_instance_tokens: HashMap::new(),
                    group_id,
                    rule_id,
                    created_at: std::time::SystemTime::now(),
                    settings_matched: true, // Default to true until checked by monitor
                },
            )
        });
    }

    /// Removes an application from the collection.
    ///
    /// # Parameters
    ///
    /// * `app_key` - The key of the application to remove
    pub fn remove_app(&mut self, app_key: &AppRuntimeKey) {
        self.mutate_membership(|apps| apps.apps.remove(app_key));
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
    fn p3_membership_token_and_logical_owner_changes_commit_once() {
        let mut apps = RunningApps::default();
        let app_key = key("Sample");
        apps.add_app(&app_key, 42, GroupId("a".into()), RuleId("a".into()));
        let before = apps.membership_generation().unwrap();
        apps.mutate_membership(|members| {
            let app = members.apps.get_mut(&app_key).unwrap();
            app.pid_instance_tokens.insert(42, 123);
            app.group_id = GroupId("b".into());
            app.rule_id = RuleId("b".into());
        });
        assert_eq!(apps.membership_generation(), Some(before + 1));
        apps.mutate_membership(|members| {
            members
                .apps
                .get_mut(&app_key)
                .unwrap()
                .pid_instance_tokens
                .insert(42, 124);
        });
        assert_eq!(apps.membership_generation(), Some(before + 2));
    }
    #[test]
    fn p3_membership_noop_settings_and_pid_order_do_not_advance() {
        let mut apps = RunningApps::default();
        let app_key = key("Sample");
        apps.add_app(&app_key, 42, GroupId("a".into()), RuleId("a".into()));
        apps.mutate_membership(|members| {
            members.apps.get_mut(&app_key).unwrap().pids.push(43);
        });
        let before = apps.membership_generation();
        apps.mutate_membership(|members| {
            let app = members.apps.get_mut(&app_key).unwrap();
            app.settings_matched = false;
            app.pids.reverse();
        });
        apps.mutate_membership(|_| {});
        assert_eq!(apps.membership_generation(), before);
        apps.remove_app(&key("Missing"));
        assert_eq!(apps.membership_generation(), before);
    }
    #[test]
    fn p3_membership_generation_overflow_is_sticky_unknown() {
        let mut apps = RunningApps::default();
        apps.membership_generation = Some(u64::MAX);
        apps.add_app(&key("Sample"), 42, GroupId("a".into()), RuleId("a".into()));
        assert_eq!(apps.membership_generation(), None);
        apps.remove_app(&key("Sample"));
        assert_eq!(apps.membership_generation(), None);
    }
    #[test]
    fn p3_membership_panicking_edit_cannot_leave_a_trusted_revision() {
        let mut apps = RunningApps::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apps.mutate_membership(|apps| {
                apps.add_app(&key("Sample"), 42, GroupId("a".into()), RuleId("a".into()));
                panic!("fixture interruption");
            })
        }));
        assert!(result.is_err());
        assert_eq!(apps.membership_generation(), None);
        apps.remove_app(&key("Sample"));
        assert_eq!(apps.membership_generation(), None);
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
        assert!(app.pid_instance_tokens.is_empty());
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
