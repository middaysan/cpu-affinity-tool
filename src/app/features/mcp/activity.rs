//! Bounded, credential-free command activity; no request arguments or output retained.
use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime},
};
pub(super) const FLASH: Duration = Duration::from_millis(180);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandStatus {
    Running,
    Queued,
    Done,
    Failed(&'static str),
    Unknown,
}
impl CommandStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Queued => "Queued",
            Self::Done => "Done",
            Self::Failed(_) => "Failed",
            Self::Unknown => "Check result",
        }
    }
    pub fn from_operation(status: &str, error: Option<&'static str>) -> Self {
        match status {
            "accepted" => Self::Queued,
            "running" => Self::Running,
            "completed" => Self::Done,
            "failed" => Self::Failed(error.unwrap_or("native")),
            _ => Self::Unknown,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct CommandEntry {
    pub sequence: u64,
    pub time: SystemTime,
    pub tool: &'static str,
    pub status: CommandStatus,
    operation: Option<String>,
}
#[derive(Default)]
pub(super) struct Journal {
    pub rows: VecDeque<CommandEntry>,
    pub revision: u64,
    pub last_received: Option<Instant>,
    last_wake: Option<Instant>,
    next: u64,
}
impl Journal {
    pub fn begin(&mut self, tool: &'static str, now: Instant) -> (u64, Duration) {
        self.next = self.next.wrapping_add(1);
        if self.rows.len() == 100 {
            self.rows.pop_front();
        }
        self.rows.push_back(CommandEntry {
            sequence: self.next,
            time: SystemTime::now(),
            tool,
            status: CommandStatus::Running,
            operation: None,
        });
        self.revision = self.revision.wrapping_add(1);
        self.last_received = Some(now);
        (self.next, self.wake_delay(now))
    }
    pub fn wake_delay(&mut self, now: Instant) -> Duration {
        let delay = self
            .last_wake
            .and_then(|last| FLASH.checked_sub(now.saturating_duration_since(last)))
            .unwrap_or_default();
        if delay.is_zero() {
            self.last_wake = Some(now);
        }
        delay
    }
    pub fn finish(&mut self, sequence: u64, status: CommandStatus, operation: Option<String>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.sequence == sequence) {
            row.status = status;
            row.operation = operation;
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn operation(&mut self, id: &str, status: CommandStatus) {
        let mut changed = false;
        for row in &mut self.rows {
            if row.operation.as_deref() == Some(id) && row.status != status {
                row.status = status;
                changed = true;
            }
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }
}
pub(super) fn flash_remaining(last: Option<Instant>, now: Instant) -> Option<Duration> {
    last.and_then(|last| FLASH.checked_sub(now.saturating_duration_since(last)))
        .filter(|remaining| !remaining.is_zero())
}
pub(super) fn command_label(tool: &str) -> &'static str {
    match tool {
        "get_capabilities" => "Check connection",
        "read_audit_log" => "Read operation history",
        "get_operation" => "Check command result",
        "get_app_state" => "Read app settings",
        "get_topology" => "Read CPU layout",
        "list_installed_apps" => "Find installed apps",
        "resolve_installed_app" => "Read installed app details",
        "read_app_logs" => "Read application logs",
        "list_crash_reports" => "List crash reports",
        "read_crash_report" => "Read crash report",
        "list_groups" => "List affinity groups",
        "list_rules" => "List saved apps",
        "create_group" => "Create affinity group",
        "update_group" => "Change affinity group",
        "delete_group" => "Remove affinity group",
        "move_group" => "Reorder affinity groups",
        "create_rule" => "Add saved app",
        "update_rule" => "Change saved app",
        "delete_rule" => "Remove saved app",
        "move_rule" => "Move saved app",
        "update_preferences" => "Change app settings",
        "read_cpu_current" => "Read current CPU load",
        "read_cpu_history" => "Read CPU history",
        "export_cpu_history" => "Export CPU history",
        "read_cpu_changes" => "Read CPU history updates",
        "read_cpu_contributors" => "Read process CPU usage",
        "list_processes" => "List running processes",
        "inspect_process" => "Read process details",
        "close_process" => "Request process closure",
        "focus_process" => "Focus process window",
        "list_runs" => "List agent-launched programs",
        "read_run_cpu" => "Read launched program CPU load",
        "read_run_output" => "Read program output",
        "launch_program" => "Launch program",
        "launch_saved" => "Launch saved app",
        "restart_saved" => "Restart saved app",
        "stop_run" => "Stop launched program",
        "restart_run" => "Restart launched program",
        "set_process_settings" => "Change process CPUs / priority",
        "stop_process" => "Force-stop process",
        "create_shortcut" => "Create desktop shortcut",
        "tweak_apply" => "Apply System Tweak",
        "tweak_undo" => "Undo System Tweak",
        "tweak_undo_all" => "Undo all System Tweaks",
        "confirm_display" => "Confirm display change",
        "read_system_tweaks" => "Read System Tweaks status",
        "read_tweak_module" => "Read System Tweak details",
        "list_tweak_profiles" => "List tweak profiles",
        "list_tweak_associations" => "List app tweak links",
        "tweak_refresh" => "Refresh System Tweaks status",
        "tweak_pc_checks_refresh" => "Run PC checks",
        "tweak_select_app" => "Choose app for System Tweak",
        "tweak_profile_create" => "Create tweak profile",
        "tweak_profile_set_module" => "Change tweak profile",
        "tweak_profile_disable" => "Disable tweak profile",
        "tweak_profile_delete" => "Remove tweak profile",
        "tweak_bind_rule" => "Link app to tweaks",
        "tweak_unbind_rule" => "Unlink app from tweaks",
        "tweak_activate" => "Activate tweak profile",
        "tweak_action" => "Run System Tweak action",
        _ => "Unknown command",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::mcp::{operations::ApiError, operations::Ledger};
    #[test]
    fn journal_keeps_exactly_the_last_100_received_commands_and_expired_finishes_cannot_overwrite()
    {
        let mut journal = Journal::default();
        let now = Instant::now();
        let old = journal.begin("get_app_state", now).0;
        for _ in 0..105 {
            let id = journal.begin("read_cpu_current", now).0;
            journal.finish(id, CommandStatus::Done, None);
        }
        assert_eq!(journal.rows.len(), 100);
        assert_eq!(journal.rows.front().unwrap().sequence, 7);
        assert_eq!(journal.rows.back().unwrap().sequence, 106);
        journal.finish(old, CommandStatus::Failed("native"), None);
        assert!(journal
            .rows
            .iter()
            .all(|row| row.status == CommandStatus::Done));
    }
    #[test]
    fn pulse_expires_and_idle_requests_no_repaint_and_bursts_are_coalesced() {
        let mut journal = Journal::default();
        let now = Instant::now();
        assert_eq!(flash_remaining(None, now), None);
        assert_eq!(journal.begin("get_app_state", now).1, Duration::ZERO);
        assert!(flash_remaining(journal.last_received, now).is_some());
        assert_eq!(
            journal
                .begin("get_app_state", now + Duration::from_millis(10))
                .1,
            FLASH - Duration::from_millis(10)
        );
        assert_eq!(
            flash_remaining(
                journal.last_received,
                now + FLASH + Duration::from_millis(11)
            ),
            None
        );
    }
    #[test]
    fn queued_and_retried_mutations_follow_actual_terminal_result_and_do_not_turn_reads_into_pending_actions(
    ) {
        let mut ledger = Ledger::new("epoch".into());
        let now = Instant::now();
        let a = ledger.commands.begin("create_group", now).0;
        let op = ledger
            .admit("agent", "epoch:1", "create_group", serde_json::json!({}))
            .unwrap();
        ledger
            .commands
            .finish(a, CommandStatus::Queued, Some(op.id.clone()));
        let b = ledger.commands.begin("create_group", now).0;
        ledger
            .commands
            .finish(b, CommandStatus::Queued, Some(op.id.clone()));
        ledger.running(&op.id);
        assert!(ledger
            .commands
            .rows
            .iter()
            .all(|row| row.status == CommandStatus::Running));
        ledger.finish(
            &op.id,
            Err(ApiError::invalid(
                "secret request values must not enter command history",
            )),
        );
        assert!(ledger
            .commands
            .rows
            .iter()
            .all(|row| row.status == CommandStatus::Failed("validation")));
        assert!(!format!("{:?}", ledger.commands.rows).contains("secret request values"));
    }
}
