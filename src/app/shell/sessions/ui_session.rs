#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
use crate::app::features::diagnostics::crash_reports::{CrashReportEntry, ReportSnapshot};
use crate::app::runtime::TrackedProcessSnapshot;
use crate::app::shared::ids::GroupId;
use crate::app::shell::sessions::{GroupFormSession, InstalledAppPickerSession, RuleEditorSession};
use crate::app::shell::{GroupRoute, WindowRoute};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct TrackedProcessSession {
    pub open: bool,
    pub revision: u64,
    pub rows: Vec<TrackedProcessSnapshot>,
    pub updating: bool,
    pub show_logs: bool,
    pub last_activity: Option<Instant>,
    last_attempt: Option<Instant>,
}

impl TrackedProcessSession {
    pub fn refresh_due(&self, now: Instant) -> bool {
        self.last_attempt
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(1))
    }

    pub fn publish(&mut self, now: Instant, rows: Option<Vec<TrackedProcessSnapshot>>) {
        self.last_attempt = Some(now);
        self.updating = rows.is_none();
        if let Some(rows) = rows {
            if rows != self.rows {
                self.rows = rows;
                self.revision = self.revision.wrapping_add(1);
            }
        }
    }
}

/// Transient UI state owned by the shell layer.
pub struct UiSession {
    pub(crate) tuning: crate::app::features::system_tweaks::presentation::TuningUiSession,
    pub(crate) app_settings_open: bool,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) cpu_monitor_open: bool,
    pub(crate) tracked_processes: TrackedProcessSession,
    pub current_window: WindowRoute,
    pub group_form: GroupFormSession,
    pub app_edit_state: RuleEditorSession,
    pub dropped_files: Option<Vec<PathBuf>>,
    pub file_drop_position: Option<eframe::egui::Pos2>,
    pub file_drop_hover_target: Option<GroupId>,
    pub installed_app_picker: InstalledAppPickerSession,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub crash_report_delete_confirmation: Option<CrashReportEntry>,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub crash_report_delete_confirmation_focus_pending: bool,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub crash_report_delete_saved_confirmation: Option<ReportSnapshot>,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub crash_report_delete_saved_confirmation_focus_pending: bool,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub crash_report_action_message: Option<String>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub windows_event_log_action_error: Option<String>,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub startup_setting_error: Option<String>,
}

impl UiSession {
    pub(crate) fn monitoring_modal_open(&self, cpu_graph_open: bool) -> bool {
        #[cfg(all(target_os = "windows", feature = "windows"))]
        if self.cpu_monitor_open {
            return true;
        }
        cpu_graph_open || self.tracked_processes.open
    }
    pub fn new(total_threads: usize) -> Self {
        Self {
            tuning: Default::default(),
            app_settings_open: false,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            cpu_monitor_open: false,
            tracked_processes: Default::default(),
            current_window: WindowRoute::Groups(GroupRoute::List),
            group_form: GroupFormSession {
                editing_group_id: None,
                editing_selection: None,
                core_selection: vec![false; total_threads],
                group_name: String::new(),
                run_all_enabled: false,
                last_clicked_core: None,
            },
            app_edit_state: RuleEditorSession {
                process_names: Default::default(),
                current_edit: None,
                background: None,
                background_error: None,
                target: None,
                shortcut_result: None,
            },
            dropped_files: None,
            file_drop_position: None,
            file_drop_hover_target: None,
            installed_app_picker: InstalledAppPickerSession::default(),
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_report_delete_confirmation: None,
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_report_delete_confirmation_focus_pending: false,
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_report_delete_saved_confirmation: None,
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_report_delete_saved_confirmation_focus_pending: false,
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_report_action_message: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log_action_error: None,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            startup_setting_error: None,
        }
    }

    pub fn reset_group_form(&mut self) {
        self.group_form.reset();
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        self.current_window = window;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tracked_roster_uses_the_existing_monitoring_modal_paint_policy() {
        let mut ui = super::UiSession::new(20);
        assert!(!ui.monitoring_modal_open(false));
        assert!(ui.monitoring_modal_open(true));
        ui.tracked_processes.open = true;
        assert!(ui.monitoring_modal_open(false));
    }
    use super::UiSession;
    use crate::app::shell::{GroupRoute, WindowRoute};

    #[test]
    fn tracked_process_session_distinguishes_initial_busy_from_completed_empty() {
        use std::time::{Duration, Instant};
        let mut state = UiSession::new(4);
        let now = Instant::now();
        assert!(state.tracked_processes.refresh_due(now));
        state.tracked_processes.publish(now, None);
        assert!(state.tracked_processes.updating);
        assert!(state.tracked_processes.rows.is_empty());
        assert_eq!(state.tracked_processes.revision, 0);
        assert!(!state
            .tracked_processes
            .refresh_due(now + Duration::from_millis(999)));
        assert!(state
            .tracked_processes
            .refresh_due(now + Duration::from_secs(1)));
        state
            .tracked_processes
            .publish(now + Duration::from_secs(1), Some(Vec::new()));
        assert!(!state.tracked_processes.updating);
        assert!(state.tracked_processes.rows.is_empty());
    }

    #[test]
    fn test_new_initializes_default_ui_state() {
        let state = UiSession::new(6);
        assert!(!state.app_settings_open);
        assert!(matches!(
            state.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert_eq!(state.group_form.core_selection.len(), 6);
        assert!(state.app_edit_state.current_edit.is_none());
        assert!(state.dropped_files.is_none());
        assert!(state.file_drop_hover_target.is_none());
        assert!(state.crash_report_delete_confirmation.is_none());
        assert!(!state.crash_report_delete_confirmation_focus_pending);
        assert!(state.crash_report_delete_saved_confirmation.is_none());
        assert!(!state.crash_report_delete_saved_confirmation_focus_pending);
        assert!(state.crash_report_action_message.is_none());
    }

    #[test]
    fn test_new_initializes_installed_app_picker_closed() {
        let state = UiSession::new(4);
        assert!(state.installed_app_picker.target_group_id.is_none());
        assert!(state.installed_app_picker.entries.is_empty());
        assert!(state.installed_app_picker.selected_entry_index.is_none());
    }
}
