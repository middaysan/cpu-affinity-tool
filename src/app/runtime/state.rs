use crate::app::adapters::storage::StorageAdapter;
#[cfg(all(test, not(all(target_os = "windows", feature = "windows"))))]
use crate::app::features::diagnostics::crash_reports::CrashReportManager;
#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::features::diagnostics::crash_reports::{
    prepare_report_directory, validated_report_path, CrashReportEntry, CrashReportIndexState,
    CrashReportManager, DeleteError, ReportSnapshot, REPORT_DIRECTORY_NAME,
};
#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::features::diagnostics::windows_event_log::{
    WindowsEventLogManager, WindowsEventLogPoll, WindowsEventLogState,
};
use crate::app::features::execution::{self, RuntimeRegistry};
use crate::app::features::preferences;
use crate::app::features::rules::{self, RulesContext};
use crate::app::features::shortcut::{
    create_saved_rule_shortcut, CreateRuleShortcutError, RuleShortcutPlatform,
    SystemRuleShortcutPlatform,
};
use crate::app::features::system_tweaks::{
    SaveError, Tuning, TuningRequest, TuningRuntime, TuningSnapshot,
};
use crate::app::models::cpu_schema::CpuSchema;
use crate::app::models::{
    effective_total_threads, AddAppsOutcome, AppRuntimeKey, AppStateStorage, AppStatus, AppToRun,
    LogManager, StateStorageMode,
};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::sessions::{RuleShortcutResult, ShortcutCreationRole};
use crate::app::shell::UiSession;
use crate::app::shell::{GroupRoute, WindowRoute};
use os_api::InstalledAppCatalogEntry;
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CentralProgramSnapshot {
    pub rule_id: RuleId,
    pub name: String,
    pub launch_target_detail: String,
    pub app_key: AppRuntimeKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CentralGroupSnapshot {
    pub group_id: GroupId,
    pub name: String,
    pub cores: Vec<usize>,
    pub is_hidden: bool,
    pub run_all_button: bool,
    pub programs: Vec<CentralProgramSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CentralPanelSnapshot {
    pub groups: Vec<CentralGroupSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleShortcutDisabledReason {
    NoTarget,
    DraftNotLoaded,
    SaveChangesFirst,
    NonPrimary,
    MissingGroup,
    MissingRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunSettingsShortcutButtonState {
    pub visible: bool,
    pub enabled: bool,
    pub disabled_reason: Option<RuleShortcutDisabledReason>,
    pub message: Option<String>,
}

fn shortcut_disabled(
    reason: RuleShortcutDisabledReason,
    message: impl Into<String>,
) -> RunSettingsShortcutButtonState {
    RunSettingsShortcutButtonState {
        visible: true,
        enabled: false,
        disabled_reason: Some(reason),
        message: Some(message.into()),
    }
}

fn default_shortcut_creation_role() -> ShortcutCreationRole {
    if cfg!(all(target_os = "windows", feature = "windows")) {
        ShortcutCreationRole::Primary
    } else {
        ShortcutCreationRole::Unsupported
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveRuleToGroupOutcome {
    Moved,
    SamePosition,
    MissingSourceGroup,
    MissingTargetGroup,
    MissingRule,
    DuplicateInTarget,
    LockFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRuleOutcome {
    Accepted,
    MissingGroup,
    MissingRule,
    LaunchRejected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledAppPickerRowSnapshot {
    pub entry_index: usize,
    pub name: String,
    pub detail: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InstalledAppPickerSnapshot {
    pub query: String,
    pub is_refreshing: bool,
    pub last_error: Option<String>,
    pub rows: Vec<InstalledAppPickerRowSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedProcessSnapshot {
    pub group_name: String,
    /// Executable filename reported by the OS, including its extension.
    pub process_name: String,
    pub pid: u32,
}
#[cfg(test)]
struct TestStatePersistence;
#[cfg(test)]
impl crate::app::features::system_tweaks::StatePersistence for TestStatePersistence {
    fn save(
        &mut self,
        _candidate: &mut AppStateStorage,
    ) -> Result<crate::app::features::system_tweaks::service::SavedState, SaveError> {
        Ok(crate::app::features::system_tweaks::service::SavedState::Unavailable)
    }
}

enum LegacyCompletion {
    Rule(
        crate::app::shell::sessions::RuleEditorTarget,
        Option<AppToRun>,
    ),
    Group(Option<GroupId>, String, Vec<bool>, bool),
    Import(ImportIntent, String),
}
enum ImportIntent {
    Files(GroupId, Vec<PathBuf>),
    Installed {
        group: GroupId,
        entry: InstalledAppCatalogEntry,
        selected: Option<usize>,
        query: String,
    },
}
struct PendingLegacy {
    id: u64,
    revision: u64,
    completion: Option<LegacyCompletion>,
}

/// Facade combining persisted, transient UI, and runtime tracking state.
pub struct AppState {
    pub(crate) persistent_state: Arc<RwLock<AppStateStorage>>,
    pub(crate) rules: RulesContext,
    pub(crate) ui: UiSession,
    pub(crate) runtime: RuntimeRegistry,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) cpu_load: crate::app::features::cpu_load::CpuLoadRuntime,
    pub(crate) log_manager: LogManager,
    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    #[cfg_attr(
        all(test, not(all(target_os = "windows", feature = "windows"))),
        allow(dead_code)
    )]
    pub(crate) crash_reports: CrashReportManager,
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) windows_event_log: WindowsEventLogManager,
    /// Privacy-safe session override used when disabling diagnostics cannot be
    /// persisted. It prevents the saved enabled preference from re-arming a lookup.
    #[cfg(all(target_os = "windows", feature = "windows"))]
    windows_event_log_disabled_for_session: bool,
    shortcut_creation_role: ShortcutCreationRole,
    tuning: TuningRuntime,
    tuning_editors: crate::app::features::system_tweaks::platform::registry::Registry,
    tuning_adopted_revision: u64,
    tuning_seen: Option<Arc<TuningSnapshot>>,
    pending_legacy: Option<PendingLegacy>,
    tuning_shutdown_reported:
        Option<crate::app::features::system_tweaks::platform::controller::ShutdownResult>,
    #[cfg(test)]
    save_count: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self::from_storage(StorageAdapter::load())
    }

    pub(crate) fn from_storage(storage: StorageAdapter) -> Self {
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let crash_report_directory = StorageAdapter::active_data_dir().join(REPORT_DIRECTORY_NAME);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let crash_reports = CrashReportManager::new(crash_report_directory);
        #[cfg(all(test, not(all(target_os = "windows", feature = "windows"))))]
        let crash_reports = CrashReportManager::new_inactive(PathBuf::new());
        let persistent_state = storage.shared();
        let rules = persistent_state
            .read()
            .map(|state| RulesContext::from_storage(&state))
            .unwrap_or_default();
        if let Ok(mut state) = persistent_state.write() {
            if state.rule_identities.is_none() {
                state.rule_identities = Some(rules.to_persisted_identities());
            }
        }
        let runtime = RuntimeRegistry::new();
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let cpu_load = crate::app::features::cpu_load::CpuLoadRuntime::new(
            runtime.running_apps_handle(),
            true,
            persistent_state.clone(),
        );
        let saved = persistent_state
            .read()
            .expect("newly created state lock")
            .clone();
        let registered = Tuning::from_registry(
            crate::app::features::system_tweaks::platforms::registry_with_scheduling(
                runtime.scheduling_handle(),
            ),
        );
        let tuning_editors = registered.registry().clone();
        let service = registered.headless_selected(saved);
        let tuning = TuningRuntime::start(
            if crate::app::features::system_tweaks::MODULES_ENABLED {
                service
            } else {
                service.without_modules()
            },
            runtime.running_apps_handle(),
        );
        Self {
            persistent_state,
            rules,
            ui: UiSession::new(effective_total_threads()),
            runtime,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            cpu_load,
            tuning,
            tuning_editors,
            tuning_adopted_revision: 0,
            tuning_seen: None,
            pending_legacy: None,
            tuning_shutdown_reported: None,
            log_manager: LogManager::default(),
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_reports,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log: WindowsEventLogManager::new_idle(),
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log_disabled_for_session: false,
            shortcut_creation_role: default_shortcut_creation_role(),
            #[cfg(test)]
            save_count: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        persistent_state: Arc<RwLock<AppStateStorage>>,
        total_threads: usize,
    ) -> Self {
        let rules = persistent_state
            .read()
            .map(|state| RulesContext::from_storage(&state))
            .unwrap_or_default();
        if let Ok(mut state) = persistent_state.write() {
            if state.rule_identities.is_none() {
                state.rule_identities = Some(rules.to_persisted_identities());
            }
        }
        let runtime = RuntimeRegistry::new();
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let cpu_load = crate::app::features::cpu_load::CpuLoadRuntime::new(
            runtime.running_apps_handle(),
            false,
            persistent_state.clone(),
        );
        let saved = persistent_state
            .read()
            .expect("newly created state lock")
            .clone();
        let registered = Tuning::from_registry(Default::default());
        let tuning_editors = registered.registry().clone();
        let service = registered.headless(saved, Box::new(TestStatePersistence));
        let tuning = TuningRuntime::start(
            if crate::app::features::system_tweaks::MODULES_ENABLED {
                service
            } else {
                service.without_modules()
            },
            runtime.running_apps_handle(),
        );
        Self {
            persistent_state,
            rules,
            ui: UiSession::new(total_threads),
            runtime,
            #[cfg(all(target_os = "windows", feature = "windows"))]
            cpu_load,
            tuning,
            tuning_editors,
            tuning_adopted_revision: 0,
            tuning_seen: None,
            pending_legacy: None,
            tuning_shutdown_reported: None,
            log_manager: LogManager::default(),
            #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
            crash_reports: CrashReportManager::new_idle(PathBuf::new()),
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log: WindowsEventLogManager::new_idle(),
            #[cfg(all(target_os = "windows", feature = "windows"))]
            windows_event_log_disabled_for_session: false,
            shortcut_creation_role: default_shortcut_creation_role(),
            save_count: 0,
        }
    }

    #[cfg_attr(not(feature = "windows"), allow(dead_code))]
    pub(crate) fn set_shortcut_creation_role(&mut self, role: ShortcutCreationRole) {
        self.shortcut_creation_role = role;
    }
    pub(crate) fn tuning_snapshot(&self) -> Arc<TuningSnapshot> {
        self.tuning.snapshot()
    }
    pub(crate) fn modules_enabled(&self) -> bool {
        self.tuning.modules_enabled()
    }
    pub(crate) fn tuning_editors(
        &self,
    ) -> &crate::app::features::system_tweaks::platform::registry::Registry {
        &self.tuning_editors
    }
    #[cfg(test)]
    pub(crate) fn replace_tuning_for_test(
        &mut self,
        runtime: TuningRuntime,
        editors: crate::app::features::system_tweaks::platform::registry::Registry,
    ) {
        let snapshot = runtime.snapshot();
        *self.persistent_state.write().unwrap() = (*snapshot.state).clone();
        self.rules = snapshot.rules.clone();
        self.tuning_adopted_revision = snapshot.revision;
        self.tuning_seen = None;
        self.tuning = runtime;
        self.tuning_editors = editors;
    }
    pub(crate) fn request_tuning(&mut self, request: TuningRequest) -> Result<u64, SaveError> {
        self.tuning.request(request)
    }
    pub(crate) fn poll_tuning(&mut self) -> bool {
        let snapshot = self.tuning.snapshot();
        let changed = self
            .tuning_seen
            .as_ref()
            .is_none_or(|seen| !Arc::ptr_eq(seen, &snapshot));
        if snapshot.revision > self.tuning_adopted_revision {
            let Ok(mut projection) = self.persistent_state.try_write() else {
                return false;
            };
            *projection = (*snapshot.state).clone();
            self.rules = snapshot.rules.clone();
            self.tuning_adopted_revision = snapshot.revision;
            #[cfg(test)]
            {
                self.save_count += 1;
            }
        }
        if let Some(outcome) = snapshot.shutdown_outcome {
            if self.tuning_shutdown_reported != Some(outcome) {
                if let Some(message) = crate::app::shell::events::ShellEvent::Tuning(
                    crate::app::shell::events::TuningDiagnostic::shutdown(outcome),
                )
                .tuning_log_message()
                {
                    self.log_manager.add_important_sticky_once(message);
                }
                self.tuning_shutdown_reported = Some(outcome);
            }
        }
        if self.pending_legacy.as_ref().is_some_and(|pending| !matches!(snapshot.operation, crate::app::features::system_tweaks::TuningOperationState::Pending { id } if id == pending.id)) {
            let pending = self.pending_legacy.take().unwrap();
            if matches!(snapshot.operation, crate::app::features::system_tweaks::TuningOperationState::Idle) && snapshot.revision > pending.revision {
                match pending.completion {
                    Some(LegacyCompletion::Rule(target, draft)) if self.ui.app_edit_state.target.as_ref() == Some(&target) && self.ui.app_edit_state.current_edit == draft => self.close_app_run_settings(),
                    Some(LegacyCompletion::Group(target, name, selection, run_all)) if self.ui.group_form.editing_group_id == target && self.ui.group_form.group_name == name && self.ui.group_form.core_selection == selection && self.ui.group_form.run_all_enabled == run_all => self.cancel_group_form_session(),
                    Some(LegacyCompletion::Import(intent, message)) => {
                        match intent {
                            ImportIntent::Installed { group, entry, selected, query } if matches!(self.ui.current_window, WindowRoute::InstalledAppPicker) && self.ui.installed_app_picker.target_group_id.as_ref() == Some(&group) && self.ui.installed_app_picker.selected_entry_index == selected && self.ui.installed_app_picker.query == query && selected.and_then(|index| self.ui.installed_app_picker.entries.get(index)) == Some(&entry) => self.close_installed_app_picker(),
                            _ => {},
                        }
                        self.log_manager.add_entry(message);
                    }
                    _ => {},
                }
            } else {
                if let Some(LegacyCompletion::Import(ImportIntent::Files(group, files), _)) = pending.completion {
                    if self.ui.dropped_files.is_none() {
                        self.ui.dropped_files = Some(files);
                        self.ui.file_drop_hover_target = Some(group);
                        self.log_manager.add_important_sticky_once("App targets were not saved. The files and target group were retained for retry.".into());
                    } else {
                        self.log_manager.add_important_sticky_once(format!("App targets were not saved for group {}: {files:?}. A newer file selection was preserved; select these files again to retry.", group.0));
                    }
                } else {
                    self.log_manager.add_important_sticky_once("Settings were not saved. The draft and saved values were retained.".into());
                }
            }
        }
        self.tuning_seen = Some(snapshot);
        changed
    }
    pub(crate) fn set_tuning_cards_visible(&self, visible: bool) {
        self.tuning.set_cards_visible(visible);
    }
    pub(crate) fn tuning_poll_interval(&self) -> Option<std::time::Duration> {
        self.tuning.poll_interval()
    }
    pub(crate) fn begin_tuning_shutdown(&mut self) {
        #[cfg(all(target_os = "windows", feature = "windows"))]
        self.cpu_load.shutdown();
        self.tuning.begin_shutdown();
    }
    #[cfg(test)]
    pub(crate) fn tuning_shutdown_complete(&self) -> bool {
        self.tuning.snapshot().shutdown_complete
    }
    pub(crate) fn tuning_shutdown_outcome(
        &self,
    ) -> Option<crate::app::features::system_tweaks::platform::controller::ShutdownResult> {
        #[cfg(all(target_os = "windows", feature = "windows"))]
        if !self.cpu_load.is_stopped() {
            return None;
        }
        self.tuning.snapshot().shutdown_outcome
    }

    #[cfg(test)]
    #[cfg_attr(not(feature = "windows"), allow(dead_code))]
    pub(crate) fn shortcut_creation_role(&self) -> ShortcutCreationRole {
        self.shortcut_creation_role
    }

    #[cfg(test)]
    fn save_count(&self) -> usize {
        self.save_count
    }

    fn base_draft(&self) -> Option<(Arc<RwLock<AppStateStorage>>, u64)> {
        self.persistent_state.try_read().ok().map(|saved| {
            (
                Arc::new(RwLock::new(saved.clone())),
                self.tuning_adopted_revision,
            )
        })
    }
    fn persist_draft(
        &mut self,
        draft: Arc<RwLock<AppStateStorage>>,
        expected_revision: u64,
        mut identities: RulesContext,
        completion: Option<LegacyCompletion>,
    ) -> bool {
        let candidate = {
            let Ok(projection) = draft.read() else {
                return false;
            };
            identities.reconcile_with_storage(&projection);
            let mut candidate = projection.clone();
            candidate.mark_ready_for_current_schema_save(identities.to_persisted_identities());
            candidate
        };
        self.enqueue_legacy(
            TuningRequest::SaveBase {
                candidate: Box::new(candidate),
                expected_revision,
            },
            completion,
        )
    }
    fn enqueue_legacy(
        &mut self,
        request: TuningRequest,
        completion: Option<LegacyCompletion>,
    ) -> bool {
        let revision = self.tuning.snapshot().revision;
        let id = match self.tuning.request(request) {
            Ok(id) => id,
            Err(error) => {
                self.log_manager.add_important_sticky_once(format!("Settings were not queued: {error:?}. Saved values and the draft were retained."));
                return false;
            }
        };
        self.pending_legacy = Some(PendingLegacy {
            id,
            revision,
            completion,
        });
        if let Some(LegacyCompletion::Import(ImportIntent::Files(group, files), _)) = self
            .pending_legacy
            .as_ref()
            .and_then(|pending| pending.completion.as_ref())
        {
            if self.ui.dropped_files.as_ref() == Some(files)
                && self.ui.file_drop_hover_target.as_ref() == Some(group)
            {
                // The accepted completion now owns this input, independently of
                // pointer hover and any fresh drop received while saving.
                self.ui.dropped_files = None;
                self.ui.file_drop_position = None;
                self.ui.file_drop_hover_target = None;
            }
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn wait_for_test_save(&mut self) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while matches!(
            self.tuning.snapshot().operation,
            crate::app::features::system_tweaks::TuningOperationState::Pending { .. }
        ) {
            assert!(
                std::time::Instant::now() < until,
                "queued save did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.poll_tuning();
    }

    fn reconcile_rules(&mut self) {
        if let Ok(state) = self.persistent_state.read() {
            self.rules.reconcile_with_storage(&state);
        }
    }

    fn group_index_for_id(&mut self, group_id: &GroupId) -> Option<usize> {
        self.reconcile_rules();
        self.rules.group_index_for_id(group_id)
    }

    fn rule_indices_for_ids(
        &mut self,
        group_id: &GroupId,
        rule_id: &RuleId,
    ) -> Option<(usize, usize)> {
        self.reconcile_rules();
        let group_index = self.rules.group_index_for_id(group_id)?;
        let rule_index = self.rules.rule_index_for_id(group_index, rule_id)?;
        Some((group_index, rule_index))
    }

    pub fn build_central_panel_snapshot(&mut self) -> CentralPanelSnapshot {
        self.reconcile_rules();
        match self.persistent_state.read() {
            Ok(state) => CentralPanelSnapshot {
                groups: state
                    .groups
                    .iter()
                    .enumerate()
                    .map(|(group_index, group)| CentralGroupSnapshot {
                        group_id: self
                            .rules
                            .group_id_for_index(group_index)
                            .expect("group identity"),
                        name: group.name.clone(),
                        cores: group.cores.clone(),
                        is_hidden: group.is_hidden,
                        run_all_button: group.run_all_button,
                        programs: group
                            .programs
                            .iter()
                            .enumerate()
                            .map(|(rule_index, app)| CentralProgramSnapshot {
                                rule_id: self
                                    .rules
                                    .rule_id_for_index(group_index, rule_index)
                                    .expect("rule identity"),
                                name: app.name.clone(),
                                launch_target_detail: app.launch_target_detail(),
                                app_key: app.get_key(),
                            })
                            .collect(),
                    })
                    .collect(),
            },
            Err(_) => CentralPanelSnapshot::default(),
        }
    }

    pub fn get_group_name(&self, index: usize) -> Option<String> {
        match self.persistent_state.read() {
            Ok(state) => state.groups.get(index).map(|group| group.name.clone()),
            Err(_) => None,
        }
    }

    pub fn set_group_is_hidden(&mut self, group_id: GroupId, is_hidden: bool) {
        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return;
        };

        let Some((draft, revision)) = self.base_draft() else {
            return;
        };
        if rules::set_group_is_hidden(&draft, group_index, is_hidden) {
            let _ = self.persist_draft(draft, revision, self.rules.clone(), None);
        }
    }

    pub fn get_group_programs(&self, index: usize) -> Option<Vec<AppToRun>> {
        self.persistent_state
            .read()
            .unwrap()
            .groups
            .get(index)
            .map(|group| group.programs.clone())
    }

    pub fn get_group_program(&self, group_index: usize, program_index: usize) -> Option<AppToRun> {
        self.persistent_state
            .read()
            .unwrap()
            .groups
            .get(group_index)
            .and_then(|group| group.programs.get(program_index).cloned())
    }

    pub fn get_cpu_schema(&self) -> CpuSchema {
        self.persistent_state.read().unwrap().cpu_schema.clone()
    }

    pub fn move_group_to_index(&mut self, group_id: GroupId, target_index: usize) -> bool {
        let Some(source_index) = self.group_index_for_id(&group_id) else {
            return false;
        };

        let Some((draft, revision)) = self.base_draft() else {
            return false;
        };
        let moved = rules::move_group_to_index(&draft, source_index, target_index);
        if moved {
            let mut identities = self.rules.clone();
            identities.move_group_to_index(source_index, target_index);
            return self.persist_draft(draft, revision, identities, None);
        }
        moved
    }

    pub fn move_rule_to_group_at(
        &mut self,
        source_group_id: GroupId,
        rule_id: RuleId,
        target_group_id: GroupId,
        target_rule_index: usize,
    ) -> MoveRuleToGroupOutcome {
        self.reconcile_rules();

        let Some(source_group_index) = self.rules.group_index_for_id(&source_group_id) else {
            return MoveRuleToGroupOutcome::MissingSourceGroup;
        };
        let Some(target_group_index) = self.rules.group_index_for_id(&target_group_id) else {
            return MoveRuleToGroupOutcome::MissingTargetGroup;
        };
        let Some(source_rule_index) = self.rules.rule_index_for_id(source_group_index, &rule_id)
        else {
            return MoveRuleToGroupOutcome::MissingRule;
        };
        if !self.rules.can_move_rule_between_groups_at(
            source_group_index,
            source_rule_index,
            target_group_index,
            target_rule_index,
        ) {
            return MoveRuleToGroupOutcome::MissingRule;
        }
        if source_group_index == target_group_index
            && (target_rule_index == source_rule_index
                || target_rule_index == source_rule_index + 1)
        {
            return MoveRuleToGroupOutcome::SamePosition;
        }

        let read_result = {
            let state = match self.persistent_state.read() {
                Ok(state) => state,
                Err(_) => return MoveRuleToGroupOutcome::LockFailed,
            };
            let Some(source_group) = state.groups.get(source_group_index) else {
                return MoveRuleToGroupOutcome::MissingSourceGroup;
            };
            let Some(target_group) = state.groups.get(target_group_index) else {
                return MoveRuleToGroupOutcome::MissingTargetGroup;
            };
            let Some(moving_app) = source_group.programs.get(source_rule_index) else {
                return MoveRuleToGroupOutcome::MissingRule;
            };
            if target_rule_index > target_group.programs.len() {
                return MoveRuleToGroupOutcome::MissingRule;
            }
            let moving_key = moving_app.get_key();
            if source_group_index != target_group_index
                && target_group
                    .programs
                    .iter()
                    .any(|program| program.get_key() == moving_key)
            {
                Err(format!(
                    "Cannot move app '{}': target group '{}' already contains the same launch rule",
                    moving_app.name, target_group.name
                ))
            } else {
                Ok((
                    moving_key,
                    moving_app.name.clone(),
                    target_group.name.clone(),
                ))
            }
        };

        let (_moving_key, moving_name, target_group_name) = match read_result {
            Ok(values) => values,
            Err(message) => {
                self.log_manager.add_entry(message);
                return MoveRuleToGroupOutcome::DuplicateInTarget;
            }
        };

        if !self.enqueue_legacy(
            TuningRequest::MoveRule {
                group: source_group_id,
                rule: rule_id,
                target: target_group_id,
                index: target_rule_index,
            },
            None,
        ) {
            return MoveRuleToGroupOutcome::LockFailed;
        }
        let message = if source_group_index == target_group_index {
            format!("Reordered app '{moving_name}' in group '{target_group_name}'")
        } else {
            format!("Moved app '{moving_name}' to group '{target_group_name}'")
        };
        self.log_manager.add_entry(message);
        MoveRuleToGroupOutcome::Moved
    }

    pub fn add_selected_files_to_group(&mut self, group_id: GroupId, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return;
        };

        let group_name = self.get_group_name(group_index).unwrap_or_default();
        self.log_manager.add_entry(format!(
            "Adding app targets to group: {group_name}, paths: {paths:?}"
        ));

        let Some((draft, revision)) = self.base_draft() else {
            return;
        };
        self.ui.dropped_files = Some(paths.clone());
        self.ui.file_drop_hover_target = Some(group_id.clone());
        let intent = ImportIntent::Files(group_id, paths.clone());
        let outcome = rules::add_apps_to_group(&draft, group_index, paths);
        self.handle_add_apps_outcome(draft, revision, group_index, &group_name, outcome, intent);
    }

    pub fn add_installed_app_to_group(
        &mut self,
        group_id: GroupId,
        entry: InstalledAppCatalogEntry,
    ) -> bool {
        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return false;
        };

        let group_name = self.get_group_name(group_index).unwrap_or_default();
        let app_name = entry.name.clone();
        let Some((draft, revision)) = self.base_draft() else {
            return false;
        };
        let completion = LegacyCompletion::Import(
            ImportIntent::Installed {
                group: group_id,
                entry: entry.clone(),
                selected: self.ui.installed_app_picker.selected_entry_index,
                query: self.ui.installed_app_picker.query.clone(),
            },
            format!("Added installed app '{app_name}' to group: {group_name}"),
        );
        let outcome = rules::add_installed_app_to_group(&draft, group_index, entry);

        let queued = if outcome.added_count > 0 {
            let mut identities = self.rules.clone();
            identities.append_rules_to_group(group_index, outcome.added_count);
            self.persist_draft(draft, revision, identities, Some(completion))
        } else {
            false
        };

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding installed app '{app_name}': {err}"));
        }
        queued
    }

    pub fn consume_dropped_files_into_group(&mut self, group_id: GroupId) -> bool {
        let Some(files) = self.ui.dropped_files.clone() else {
            self.ui.file_drop_hover_target = None;
            return false;
        };

        if files.is_empty() {
            self.ui.dropped_files = None;
            self.ui.file_drop_position = None;
            self.ui.file_drop_hover_target = None;
            return false;
        }

        let Some(group_index) = self.group_index_for_id(&group_id) else {
            self.ui.dropped_files = None;
            self.ui.file_drop_position = None;
            self.ui.file_drop_hover_target = None;
            return false;
        };
        self.ui.file_drop_hover_target = Some(group_id.clone());

        let group_name = self.get_group_name(group_index).unwrap_or_default();

        let Some((draft, revision)) = self.base_draft() else {
            return false;
        };
        let outcome = rules::add_apps_to_group(&draft, group_index, files.clone());
        self.handle_add_apps_outcome(
            draft,
            revision,
            group_index,
            &group_name,
            outcome,
            ImportIntent::Files(group_id, files),
        )
    }

    fn handle_add_apps_outcome(
        &mut self,
        draft: Arc<RwLock<AppStateStorage>>,
        revision: u64,
        group_index: usize,
        group_name: &str,
        outcome: AddAppsOutcome,
        intent: ImportIntent,
    ) -> bool {
        let attempted_count = match &intent {
            ImportIntent::Files(_, files) => files.len(),
            ImportIntent::Installed { .. } => 1,
        };
        let queued = if outcome.added_count > 0 {
            let mut identities = self.rules.clone();
            identities.append_rules_to_group(group_index, outcome.added_count);
            let message = if outcome.added_count == attempted_count {
                format!("Added app targets to group: {group_name}")
            } else {
                format!(
                    "Added {} app targets to group: {}",
                    outcome.added_count, group_name
                )
            };
            self.persist_draft(
                draft,
                revision,
                identities,
                Some(LegacyCompletion::Import(intent, message)),
            )
        } else {
            if let ImportIntent::Files(group, files) = intent {
                if self.ui.dropped_files.as_ref() == Some(&files)
                    && self.ui.file_drop_hover_target.as_ref() == Some(&group)
                {
                    self.ui.dropped_files = None;
                    self.ui.file_drop_position = None;
                    self.ui.file_drop_hover_target = None;
                }
            }
            false
        };

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding app targets: {err}"));
        }
        queued
    }

    pub fn get_theme_index(&self) -> usize {
        self.persistent_state.read().unwrap().theme_index
    }

    pub fn start_app_with_autorun(&mut self) {
        execution::start_app_with_autorun(
            &self.persistent_state,
            &self.runtime,
            &mut self.log_manager,
        );
    }

    pub fn set_theme_index(&mut self, theme_index: usize) {
        let Some((draft, revision)) = self.base_draft() else {
            return;
        };
        if preferences::set_theme_index(&draft, theme_index) {
            let _ = self.persist_draft(draft, revision, self.rules.clone(), None);
        }
    }

    pub fn toggle_process_monitoring(&mut self) {
        let Some((draft, revision)) = self.base_draft() else {
            return;
        };
        preferences::toggle_process_monitoring(&draft);
        let _ = self.persist_draft(draft, revision, self.rules.clone(), None);
    }

    pub fn is_process_monitoring_enabled(&self) -> bool {
        self.persistent_state
            .read()
            .unwrap()
            .process_monitoring_enabled
    }

    pub fn commit_group_form_session(&mut self) {
        let Some((draft, revision)) = self.base_draft() else {
            return;
        };
        let completion = LegacyCompletion::Group(
            self.ui.group_form.editing_group_id.clone(),
            self.ui.group_form.group_name.clone(),
            self.ui.group_form.core_selection.clone(),
            self.ui.group_form.run_all_enabled,
        );
        let should_save = if let Some(group_id) = self.ui.group_form.editing_group_id.clone() {
            let Some(index) = self.group_index_for_id(&group_id) else {
                self.ui.reset_group_form();
                self.ui
                    .set_current_window(WindowRoute::Groups(GroupRoute::List));
                return;
            };

            match rules::update_group_properties(
                &draft,
                index,
                self.ui.group_form.group_name.clone(),
                &self.ui.group_form.core_selection,
                self.ui.group_form.run_all_enabled,
            ) {
                Ok(updated) => updated,
                Err(err) => {
                    self.log_manager.add_entry(err);
                    false
                }
            }
        } else {
            match rules::create_group(
                &draft,
                &self.ui.group_form.group_name,
                &self.ui.group_form.core_selection,
                self.ui.group_form.run_all_enabled,
            ) {
                Ok(()) => true,
                Err(err) => {
                    self.log_manager.add_entry(err);
                    false
                }
            }
        };

        if should_save {
            let mut identities = self.rules.clone();
            if self.ui.group_form.editing_group_id.is_none() {
                identities.append_group();
            }
            let _ = self.persist_draft(draft, revision, identities, Some(completion));
        }
    }

    pub fn delete_current_group_form_target(&mut self) {
        if let Some(group_id) = self.ui.group_form.editing_group_id.clone() {
            let completion = LegacyCompletion::Group(
                Some(group_id.clone()),
                self.ui.group_form.group_name.clone(),
                self.ui.group_form.core_selection.clone(),
                self.ui.group_form.run_all_enabled,
            );
            let _ = self.enqueue_legacy(
                TuningRequest::RemoveGroup { group: group_id },
                Some(completion),
            );
        }
    }

    pub fn cancel_group_form_session(&mut self) {
        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        let window = if matches!(window, WindowRoute::Tuning) && !self.modules_enabled() {
            WindowRoute::Groups(GroupRoute::List)
        } else {
            window
        };
        let leaving_installed_app_picker =
            matches!(self.ui.current_window, WindowRoute::InstalledAppPicker)
                && !matches!(window, WindowRoute::InstalledAppPicker);
        let leaving_app_run_settings =
            matches!(self.ui.current_window, WindowRoute::AppRunSettings)
                && !matches!(window, WindowRoute::AppRunSettings);
        #[cfg(all(target_os = "windows", feature = "windows"))]
        let leaving_crash_reports = matches!(self.ui.current_window, WindowRoute::CrashReports)
            && !matches!(window, WindowRoute::CrashReports);

        if leaving_installed_app_picker {
            self.reset_installed_app_picker_session();
        }
        if leaving_app_run_settings {
            self.reset_app_run_settings_session();
        }
        #[cfg(all(target_os = "windows", feature = "windows"))]
        if leaving_crash_reports {
            self.ui.crash_report_delete_confirmation = None;
            self.ui.crash_report_delete_confirmation_focus_pending = false;
            self.ui.crash_report_delete_saved_confirmation = None;
            self.ui.crash_report_delete_saved_confirmation_focus_pending = false;
            self.ui.crash_report_action_message = None;
        }

        self.ui.set_current_window(window);
    }

    pub fn start_creating_group(&mut self) {
        self.ui.reset_group_form();
        self.set_current_window(WindowRoute::Groups(GroupRoute::Create));
    }

    pub fn start_editing_group(&mut self, group_id: GroupId) {
        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return;
        };

        if let Some(group) = rules::load_group_for_edit(&self.persistent_state, group_index) {
            let total_cores = self.ui.group_form.core_selection.len();
            let mut selection = vec![false; total_cores];
            for core in group.selected_cores {
                if core < total_cores {
                    selection[core] = true;
                }
            }
            self.ui.group_form.core_selection = selection;
            self.ui.group_form.group_name = group.name;
            self.ui.group_form.run_all_enabled = group.run_all_enabled;
            self.ui.group_form.last_clicked_core = None;
            self.ui.current_window = WindowRoute::Groups(GroupRoute::Edit);
        } else {
            self.log_manager
                .add_entry(format!("Group with index {group_index} not found"));
            return;
        }
        self.ui.group_form.editing_group_id = Some(group_id);
    }

    fn run_app_with_affinity_sync(
        &mut self,
        group_index: usize,
        prog_index: usize,
        app_to_run: AppToRun,
    ) -> execution::LaunchDispatchOutcome {
        execution::run_app_with_affinity_sync(
            &self.persistent_state,
            &self.runtime,
            &mut self.log_manager,
            group_index,
            prog_index,
            app_to_run,
        )
    }

    pub fn run_group_program(&mut self, group_id: GroupId, rule_id: RuleId) -> RunRuleOutcome {
        self.reconcile_rules();
        let Some(group_index) = self.rules.group_index_for_id(&group_id) else {
            return RunRuleOutcome::MissingGroup;
        };
        let Some(program_index) = self.rules.rule_index_for_id(group_index, &rule_id) else {
            return RunRuleOutcome::MissingRule;
        };

        if let Some(app_to_run) = self.get_group_program(group_index, program_index) {
            match self.run_app_with_affinity_sync(group_index, program_index, app_to_run) {
                execution::LaunchDispatchOutcome::Accepted => RunRuleOutcome::Accepted,
                execution::LaunchDispatchOutcome::Rejected(message) => {
                    RunRuleOutcome::LaunchRejected(message)
                }
            }
        } else {
            RunRuleOutcome::MissingRule
        }
    }

    pub fn run_group_program_action(
        &mut self,
        group_id: GroupId,
        rule_id: RuleId,
        action: execution::AppRowAction,
    ) -> RunRuleOutcome {
        self.reconcile_rules();
        let Some(group_index) = self.rules.group_index_for_id(&group_id) else {
            return RunRuleOutcome::MissingGroup;
        };
        let Some(program_index) = self.rules.rule_index_for_id(group_index, &rule_id) else {
            return RunRuleOutcome::MissingRule;
        };

        if let Some(app_to_run) = self.get_group_program(group_index, program_index) {
            match execution::run_app_row_action(
                &self.persistent_state,
                &mut self.runtime,
                &mut self.log_manager,
                execution::AppRowActionRequest {
                    group_index,
                    program_index,
                    app: app_to_run,
                    action,
                },
            ) {
                execution::LaunchDispatchOutcome::Accepted => RunRuleOutcome::Accepted,
                execution::LaunchDispatchOutcome::Rejected(message) => {
                    RunRuleOutcome::LaunchRejected(message)
                }
            }
        } else {
            RunRuleOutcome::MissingRule
        }
    }

    pub fn create_shortcut_for_rule_with_platform(
        &mut self,
        group_id: GroupId,
        rule_id: RuleId,
        platform: &mut impl RuleShortcutPlatform,
    ) -> Result<PathBuf, CreateRuleShortcutError> {
        self.reconcile_rules();
        let result = match self.persistent_state.read() {
            Ok(storage) => {
                create_saved_rule_shortcut(&storage, &self.rules, group_id, rule_id, platform)
            }
            Err(_) => Err(CreateRuleShortcutError::StorageUnavailable),
        };

        match &result {
            Ok(path) => self
                .log_manager
                .add_entry(format!("Shortcut created: {}", path.display())),
            Err(err) => self.log_manager.add_important_entry(format!(
                "Shortcut creation failed: {}",
                err.to_user_message()
            )),
        }

        result
    }

    pub(crate) fn clear_current_app_shortcut_result(&mut self) {
        self.ui.app_edit_state.shortcut_result = None;
    }

    pub(crate) fn is_current_app_edit_dirty(&mut self) -> bool {
        let (Some(target), Some(current_edit)) = (
            self.ui.app_edit_state.target.clone(),
            self.ui.app_edit_state.current_edit.clone(),
        ) else {
            return false;
        };

        let Some((group_idx, prog_idx)) =
            self.rule_indices_for_ids(&target.group_id, &target.rule_id)
        else {
            return false;
        };

        rules::load_rule(&self.persistent_state, group_idx, prog_idx)
            .is_none_or(|saved| saved != current_edit)
    }

    pub(crate) fn current_app_edit_shortcut_status(&mut self) -> RunSettingsShortcutButtonState {
        if !cfg!(all(target_os = "windows", feature = "windows")) {
            return RunSettingsShortcutButtonState {
                visible: false,
                enabled: false,
                disabled_reason: None,
                message: None,
            };
        }

        let Some(target) = self.ui.app_edit_state.target.clone() else {
            return shortcut_disabled(
                RuleShortcutDisabledReason::NoTarget,
                "Shortcut target is unavailable.",
            );
        };
        if self.ui.app_edit_state.current_edit.is_none() {
            return shortcut_disabled(
                RuleShortcutDisabledReason::DraftNotLoaded,
                "Shortcut target is not loaded.",
            );
        }

        match self.shortcut_creation_role {
            ShortcutCreationRole::Primary => {}
            ShortcutCreationRole::NonPrimary => {
                return shortcut_disabled(
                    RuleShortcutDisabledReason::NonPrimary,
                    "Close the other running instance first.",
                );
            }
            ShortcutCreationRole::Unsupported => {
                return RunSettingsShortcutButtonState {
                    visible: false,
                    enabled: false,
                    disabled_reason: None,
                    message: None,
                };
            }
        }

        self.reconcile_rules();
        let Some(group_idx) = self.rules.group_index_for_id(&target.group_id) else {
            return shortcut_disabled(
                RuleShortcutDisabledReason::MissingGroup,
                "Shortcut target group was not found.",
            );
        };
        if self
            .rules
            .rule_index_for_id(group_idx, &target.rule_id)
            .is_none()
        {
            return shortcut_disabled(
                RuleShortcutDisabledReason::MissingRule,
                "Shortcut target rule was not found.",
            );
        }

        if self.is_current_app_edit_dirty() {
            return shortcut_disabled(
                RuleShortcutDisabledReason::SaveChangesFirst,
                "Save changes first.",
            );
        }

        RunSettingsShortcutButtonState {
            visible: true,
            enabled: true,
            disabled_reason: None,
            message: None,
        }
    }

    pub(crate) fn create_shortcut_for_current_rule(
        &mut self,
    ) -> Result<PathBuf, CreateRuleShortcutError> {
        let mut platform = SystemRuleShortcutPlatform;
        self.create_shortcut_for_current_rule_with_platform(&mut platform)
    }

    pub(crate) fn create_shortcut_for_current_rule_with_platform(
        &mut self,
        platform: &mut impl RuleShortcutPlatform,
    ) -> Result<PathBuf, CreateRuleShortcutError> {
        let status = self.current_app_edit_shortcut_status();
        if !status.visible {
            let err = CreateRuleShortcutError::UnsupportedPlatform;
            self.ui.app_edit_state.shortcut_result = Some(RuleShortcutResult::Failed {
                message: err.to_user_message().to_string(),
            });
            return Err(err);
        }
        if !status.enabled {
            let err = match status.disabled_reason {
                Some(RuleShortcutDisabledReason::NoTarget) => CreateRuleShortcutError::NoTarget,
                Some(RuleShortcutDisabledReason::DraftNotLoaded) => {
                    CreateRuleShortcutError::DraftNotLoaded
                }
                Some(RuleShortcutDisabledReason::SaveChangesFirst) => {
                    CreateRuleShortcutError::SaveChangesFirst
                }
                Some(RuleShortcutDisabledReason::NonPrimary) => {
                    CreateRuleShortcutError::UnsupportedPlatform
                }
                Some(RuleShortcutDisabledReason::MissingGroup) => {
                    CreateRuleShortcutError::MissingGroup
                }
                Some(RuleShortcutDisabledReason::MissingRule) => {
                    CreateRuleShortcutError::MissingRule
                }
                None => CreateRuleShortcutError::NoTarget,
            };
            self.ui.app_edit_state.shortcut_result = Some(RuleShortcutResult::Failed {
                message: if matches!(
                    status.disabled_reason,
                    Some(RuleShortcutDisabledReason::NonPrimary)
                ) {
                    status
                        .message
                        .unwrap_or_else(|| err.to_user_message().to_string())
                } else {
                    err.to_user_message().to_string()
                },
            });
            return Err(err);
        }

        let Some(target) = self.ui.app_edit_state.target.clone() else {
            return Err(CreateRuleShortcutError::NoTarget);
        };
        let result =
            self.create_shortcut_for_rule_with_platform(target.group_id, target.rule_id, platform);
        self.ui.app_edit_state.shortcut_result = Some(match &result {
            Ok(path) => RuleShortcutResult::Created {
                filename: path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string()),
            },
            Err(err) => RuleShortcutResult::Failed {
                message: err.to_user_message().to_string(),
            },
        });

        result
    }

    pub fn run_group(&mut self, group_id: GroupId) {
        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return;
        };

        let Some(programs) = self.get_group_programs(group_index) else {
            return;
        };

        if programs.is_empty() {
            let group_name = self.get_group_name(group_index).unwrap_or_default();
            self.log_manager
                .add_entry(format!("No app targets to run in group: {group_name}"));
            return;
        }

        for (program_index, program) in programs.into_iter().enumerate() {
            self.run_app_with_affinity_sync(group_index, program_index, program);
        }
    }

    pub fn get_app_status_sync(&mut self, app_key: &AppRuntimeKey) -> AppStatus {
        self.runtime.get_app_status_sync(app_key)
    }

    pub fn get_running_app_pids(&self, app_key: &AppRuntimeKey) -> Option<Vec<u32>> {
        self.runtime.get_running_app_pids(app_key)
    }

    pub fn start_minimized(&self) -> bool {
        self.persistent_state
            .read()
            .map(|state| state.start_minimized)
            .unwrap_or(false)
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn cpu_load_enabled(&self) -> bool {
        self.cpu_load.preference_override.unwrap_or_else(|| {
            self.persistent_state
                .read()
                .map(|s| s.cpu_load_monitoring_enabled)
                .unwrap_or(false)
        })
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn cpu_load_history_seconds(&self) -> u32 {
        self.cpu_load.history_seconds_override.unwrap_or_else(|| {
            self.persistent_state
                .read()
                .map(|state| {
                    AppStateStorage::normalize_cpu_load_history_seconds(
                        state.cpu_load_history_seconds,
                    )
                })
                .unwrap_or(300)
        })
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn set_cpu_load_history_seconds(&mut self, seconds: u32) -> Result<(), String> {
        if !matches!(seconds, 30 | 300 | 1800) {
            return Err(
                "Choose a CPU history duration of 30 seconds, 5 minutes or 30 minutes.".into(),
            );
        }
        if self.pending_legacy.is_some()
            || matches!(
                self.tuning.snapshot().operation,
                crate::app::features::system_tweaks::TuningOperationState::Pending { .. }
            )
        {
            return Err(
                "Wait for the current settings change to finish, then change CPU history.".into(),
            );
        }
        let (draft, revision) = self
            .base_draft()
            .ok_or("Could not update CPU history. Please try again.")?;
        draft
            .write()
            .map_err(|_| "Could not update CPU history.")?
            .cpu_load_history_seconds = seconds;
        if !self.persist_draft(draft, revision, self.rules.clone(), None) {
            return Err("Could not save CPU history. Please try again.".into());
        }
        self.cpu_load.history_seconds_override = Some(seconds);
        self.cpu_load.preference_error = None;
        self.cpu_load.set_history_seconds(seconds);
        Ok(())
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn set_cpu_load_enabled(&mut self, enabled: bool) -> Result<(), String> {
        // Disabling takes effect now, even if persistence is unavailable.
        if !enabled {
            self.cpu_load.preference_override = Some(false);
            self.cpu_load.configure(false, false);
        }
        let (draft, revision) = self
            .base_draft()
            .ok_or("Could not update CPU monitoring. Please try again.")?;
        draft
            .write()
            .map_err(|_| "Could not update CPU monitoring.")?
            .cpu_load_monitoring_enabled = enabled;
        if !self.persist_draft(draft, revision, self.rules.clone(), None) {
            return Err("Could not save CPU monitoring. Please try again.".into());
        }
        self.cpu_load.preference_override = Some(enabled);
        Ok(())
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn poll_cpu_load(&mut self, visible: bool) {
        if self.pending_legacy.is_none() {
            if let Some(wanted) = self.cpu_load.preference_override {
                let saved = self
                    .persistent_state
                    .read()
                    .map(|s| s.cpu_load_monitoring_enabled)
                    .unwrap_or(false);
                if saved == wanted {
                    self.cpu_load.preference_override = None;
                } else {
                    self.cpu_load.preference_error = Some("CPU monitoring changed for this session, but could not be saved. Try again in App settings.".into());
                }
            }
            if let Some(wanted) = self.cpu_load.history_seconds_override {
                let saved = self
                    .persistent_state
                    .read()
                    .map(|state| state.cpu_load_history_seconds)
                    .unwrap_or(300);
                if saved == wanted {
                    self.cpu_load.history_seconds_override = None;
                } else {
                    self.cpu_load.preference_error = Some("CPU history changed for this session, but could not be saved. Try again in App settings.".into());
                }
            }
        }
        let enabled = self.cpu_load_enabled();
        let history_seconds = self.cpu_load_history_seconds();
        self.cpu_load.set_history_seconds(history_seconds);
        self.cpu_load.configure(enabled, visible);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn cpu_load_memory_estimate(&self) -> String {
        let apps = self
            .persistent_state
            .read()
            .map(|s| s.groups.iter().map(|g| g.programs.len()).sum())
            .unwrap_or_default();
        self.cpu_load.memory_estimate(
            self.ui.group_form.core_selection.len(),
            apps,
            self.cpu_load_history_seconds(),
        )
    }

    #[cfg(any(test, all(target_os = "windows", feature = "windows")))]
    pub fn set_start_minimized(&mut self, enabled: bool) -> Result<(), String> {
        let (draft, revision) = self
            .base_draft()
            .ok_or_else(|| "Could not update the startup preference.".to_string())?;
        {
            let mut state = draft
                .write()
                .map_err(|_| "Could not update the startup preference.".to_string())?;
            let previous = state.start_minimized;
            if previous == enabled {
                return Ok(());
            }
            state.start_minimized = enabled;
        }
        if !self.persist_draft(draft, revision, self.rules.clone(), None) {
            return Err("Could not save Start minimized. Please try again.".to_string());
        }
        Ok(())
    }

    pub(crate) fn refresh_tracked_processes(&mut self, now: std::time::Instant) {
        self.refresh_tracked_processes_with_inspector(
            now,
            crate::app::adapters::os::get_process_image_path_and_instance_token,
        );
    }

    fn refresh_tracked_processes_with_inspector(
        &mut self,
        now: std::time::Instant,
        inspect: impl FnMut(u32) -> Result<(PathBuf, u64), String>,
    ) {
        if self.ui.tracked_processes.refresh_due(now) {
            let rows = self.tracked_processes_snapshot_with_inspector(inspect);
            self.ui.tracked_processes.publish(now, rows);
        }
    }

    /// Revalidates only tracked instances; None means temporarily unavailable,
    /// whereas Some(empty) is a completed empty roster.
    fn tracked_processes_snapshot_with_inspector(
        &mut self,
        mut inspect: impl FnMut(u32) -> Result<(PathBuf, u64), String>,
    ) -> Option<Vec<TrackedProcessSnapshot>> {
        if self.persistent_state.is_poisoned() {
            return None;
        }
        let central = self.build_central_panel_snapshot();
        let handle = self.runtime.running_apps_handle();
        let instances = {
            let Ok(running) = handle.try_read() else {
                return None;
            };
            let mut instances = Vec::new();
            for group in central.groups {
                for program in group.programs {
                    let Some(app) = running.apps.get(&program.app_key) else {
                        continue;
                    };
                    if app.group_id != group.group_id || app.rule_id != program.rule_id {
                        continue;
                    }
                    let mut pids = app.pids.clone();
                    pids.sort_unstable();
                    pids.dedup();
                    for pid in pids {
                        if let Some(&token) = app.pid_instance_tokens.get(&pid) {
                            instances.push((group.name.clone(), pid, token));
                        }
                    }
                }
            }
            instances
        };
        let mut tracked = Vec::new();
        for (group_name, pid, tracked_token) in instances {
            // Inspect after releasing the runtime lock. The name and token must
            // describe the same instance, not an unrelated process reusing its PID.
            let Ok((path, current_token)) = inspect(pid) else {
                continue;
            };
            if current_token != tracked_token {
                continue;
            }
            let Some(process_name) = path.file_name() else {
                continue;
            };
            tracked.push(TrackedProcessSnapshot {
                group_name,
                process_name: process_name.to_string_lossy().into_owned(),
                pid,
            });
        }

        Some(tracked)
    }

    pub fn open_installed_app_picker(&mut self, group_id: GroupId) {
        let picker = &mut self.ui.installed_app_picker;
        picker.target_group_id = Some(group_id);
        picker.query.clear();
        picker.last_error = None;
        picker.needs_focus = true;
        picker.selected_entry_index = picker.entries.first().map(|_| 0);
        self.normalize_installed_app_picker_selection();
        self.ui.set_current_window(WindowRoute::InstalledAppPicker);
        self.request_installed_app_picker_refresh();
    }

    pub fn close_installed_app_picker(&mut self) {
        self.reset_installed_app_picker_session();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn build_installed_app_picker_snapshot(&self) -> InstalledAppPickerSnapshot {
        let picker = &self.ui.installed_app_picker;
        let rows = self
            .filtered_installed_app_entry_indices()
            .into_iter()
            .map(|entry_index| {
                let entry = &picker.entries[entry_index];
                let detail = if entry.detail.trim().is_empty() {
                    entry.source.label().to_string()
                } else {
                    format!("{} • {}", entry.source.label(), entry.detail)
                };

                InstalledAppPickerRowSnapshot {
                    entry_index,
                    name: entry.name.clone(),
                    detail,
                    selected: picker.selected_entry_index == Some(entry_index),
                }
            })
            .collect();

        InstalledAppPickerSnapshot {
            query: picker.query.clone(),
            is_refreshing: picker.is_refreshing,
            last_error: picker.last_error.clone(),
            rows,
        }
    }

    pub fn set_installed_app_picker_query(&mut self, query: String) {
        self.ui.installed_app_picker.query = query;
        self.normalize_installed_app_picker_selection();
    }

    pub fn select_installed_app_picker_entry(&mut self, entry_index: usize) {
        self.ui.installed_app_picker.selected_entry_index = Some(entry_index);
    }

    pub fn select_next_installed_app_picker_entry(&mut self) {
        let filtered = self.filtered_installed_app_entry_indices();
        if filtered.is_empty() {
            self.ui.installed_app_picker.selected_entry_index = None;
            return;
        }

        let picker = &mut self.ui.installed_app_picker;
        let next_position = picker
            .selected_entry_index
            .and_then(|selected| filtered.iter().position(|&idx| idx == selected))
            .map(|pos| (pos + 1).min(filtered.len() - 1))
            .unwrap_or(0);
        picker.selected_entry_index = Some(filtered[next_position]);
    }

    pub fn select_previous_installed_app_picker_entry(&mut self) {
        let filtered = self.filtered_installed_app_entry_indices();
        if filtered.is_empty() {
            self.ui.installed_app_picker.selected_entry_index = None;
            return;
        }

        let picker = &mut self.ui.installed_app_picker;
        let prev_position = picker
            .selected_entry_index
            .and_then(|selected| filtered.iter().position(|&idx| idx == selected))
            .map(|pos| pos.saturating_sub(1))
            .unwrap_or(0);
        picker.selected_entry_index = Some(filtered[prev_position]);
    }

    pub fn request_installed_app_picker_refresh(&mut self) {
        if self.ui.installed_app_picker.is_refreshing {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.ui.installed_app_picker.is_refreshing = true;
        self.ui.installed_app_picker.last_error = None;
        self.ui.installed_app_picker.refresh_rx = Some(rx);

        let wake = self.runtime.wake_handle();
        std::thread::spawn(move || {
            let _ = tx.send(crate::app::adapters::discovery::list_supported_start_apps());
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    pub fn poll_installed_app_picker_refresh(&mut self) {
        let Some(rx) = self.ui.installed_app_picker.refresh_rx.take() else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.ui.installed_app_picker.is_refreshing = false;

                let previous_selection = self
                    .ui
                    .installed_app_picker
                    .selected_entry_index
                    .and_then(|idx| self.ui.installed_app_picker.entries.get(idx).cloned());

                match result {
                    Ok(entries) => {
                        self.ui.installed_app_picker.entries = entries;
                        self.ui.installed_app_picker.last_error = None;
                        self.ui.installed_app_picker.selected_entry_index = previous_selection
                            .and_then(|selected| {
                                self.ui
                                    .installed_app_picker
                                    .entries
                                    .iter()
                                    .position(|entry| entry == &selected)
                            });
                    }
                    Err(err) => {
                        self.log_manager
                            .add_entry(format!("Installed app refresh failed: {err}"));
                        self.ui.installed_app_picker.last_error = Some(err);
                    }
                }

                self.normalize_installed_app_picker_selection();
            }
            Err(TryRecvError::Empty) => {
                self.ui.installed_app_picker.refresh_rx = Some(rx);
            }
            Err(TryRecvError::Disconnected) => {
                self.ui.installed_app_picker.is_refreshing = false;
                self.log_manager
                    .add_entry("Installed app refresh channel disconnected".into());
                self.ui.installed_app_picker.last_error =
                    Some("Installed app refresh channel disconnected".into());
            }
        }
    }

    pub fn take_installed_app_picker_focus_request(&mut self) -> bool {
        if self.ui.installed_app_picker.needs_focus {
            self.ui.installed_app_picker.needs_focus = false;
            true
        } else {
            false
        }
    }

    pub fn confirm_selected_installed_app(&mut self) -> bool {
        let Some(group_id) = self.ui.installed_app_picker.target_group_id.clone() else {
            return false;
        };

        let Some(entry_index) = self
            .ui
            .installed_app_picker
            .selected_entry_index
            .or_else(|| self.filtered_installed_app_entry_indices().first().copied())
        else {
            return false;
        };

        let Some(entry) = self
            .ui
            .installed_app_picker
            .entries
            .get(entry_index)
            .cloned()
        else {
            return false;
        };

        self.ui.installed_app_picker.selected_entry_index = Some(entry_index);
        self.add_installed_app_to_group(group_id, entry)
    }

    pub fn open_app_run_settings(&mut self, group_id: GroupId, rule_id: RuleId) {
        self.ui.app_edit_state.current_edit = None;
        self.ui.app_edit_state.target =
            Some(crate::app::shell::sessions::RuleEditorTarget { group_id, rule_id });
        self.ui.app_edit_state.shortcut_result = None;
        self.ui.current_window = WindowRoute::AppRunSettings;
    }

    pub fn close_app_run_settings(&mut self) {
        self.reset_app_run_settings_session();
        self.ui.current_window = WindowRoute::Groups(GroupRoute::List);
    }

    pub fn ensure_current_edit_loaded(&mut self) -> bool {
        let Some(target) = self.ui.app_edit_state.target.clone() else {
            self.close_app_run_settings();
            return false;
        };

        let Some((group_idx, prog_idx)) =
            self.rule_indices_for_ids(&target.group_id, &target.rule_id)
        else {
            self.close_app_run_settings();
            return false;
        };

        if self.ui.app_edit_state.current_edit.is_none() {
            if let Some(original) = rules::load_rule(&self.persistent_state, group_idx, prog_idx) {
                self.ui.app_edit_state.current_edit = Some(original);
            } else {
                self.close_app_run_settings();
                return false;
            }
        }

        true
    }

    pub fn commit_current_app_edit_session(&mut self) {
        if let (Some(target), Some(updated_app)) = (
            self.ui.app_edit_state.target.clone(),
            self.ui.app_edit_state.current_edit.clone(),
        ) {
            if let Some((group_idx, prog_idx)) =
                self.rule_indices_for_ids(&target.group_id, &target.rule_id)
            {
                let mut updated_app = updated_app;
                if let Some(original) =
                    rules::load_rule(&self.persistent_state, group_idx, prog_idx)
                {
                    updated_app.sync_primary_process_name_after_path_edit(&original);
                }

                let completion = LegacyCompletion::Rule(
                    target.clone(),
                    self.ui.app_edit_state.current_edit.clone(),
                );
                let _ = self.enqueue_legacy(
                    TuningRequest::EditRule {
                        group: target.group_id,
                        rule: target.rule_id,
                        app: Box::new(updated_app),
                    },
                    Some(completion),
                );
            }
        }
    }

    pub fn delete_current_app_edit_target(&mut self) {
        if let Some(target) = self.ui.app_edit_state.target.clone() {
            let label = self
                .rule_indices_for_ids(&target.group_id, &target.rule_id)
                .and_then(|(group, rule)| self.get_group_program(group, rule))
                .map(|app| app.launch_target_label());
            let completion =
                LegacyCompletion::Rule(target.clone(), self.ui.app_edit_state.current_edit.clone());
            if self.enqueue_legacy(
                TuningRequest::RemoveRule {
                    group: target.group_id,
                    rule: target.rule_id,
                },
                Some(completion),
            ) {
                if let Some(label) = label {
                    self.log_manager
                        .add_entry(format!("Queued app removal: {label}"));
                }
            }
        }
    }

    pub fn clear_logs(&mut self) {
        self.log_manager.clear();
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn set_windows_event_log_diagnostics_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), String> {
        // Invalidate visible evidence and worker/retry state before even
        // attempting to read or write persistent state. A poisoned lock or a
        // failed disk write must not keep diagnostics active.
        if !enabled {
            self.windows_event_log.disable();
            self.windows_event_log_disabled_for_session = true;
        }
        {
            let state = match self.persistent_state.read() {
                Ok(state) => state,
                Err(_) => {
                    return Err(
                        "could not read the diagnostics preference; diagnostics remain disabled for this session"
                            .to_string(),
                    )
                }
            };
            let _ = state.windows_event_log_diagnostics_enabled;
        }
        let (draft, revision) = self
            .base_draft()
            .ok_or_else(|| "could not read the diagnostics preference".to_string())?;
        if let Err(error) = preferences::set_windows_event_log_diagnostics(&draft, enabled) {
            return Err(format!(
                "{error}; diagnostics remain disabled for this session"
            ));
        }
        if !self.persist_draft(draft, revision, self.rules.clone(), None) {
            if enabled {
                return Err("could not save the Windows Event Log diagnostics setting; diagnostics remain disabled for this session".to_string());
            }
            return Err("Windows Event Log diagnostics disabled for this session, but the setting could not be saved".to_string());
        }

        if enabled {
            self.windows_event_log_disabled_for_session = false;
            self.windows_event_log.enable();
        }
        Ok(())
    }

    /// Called by the shell only after the first frame has been rendered.
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn start_windows_event_log_scan_after_shell_gate(&mut self) -> bool {
        if self.windows_event_log_disabled_for_session {
            return false;
        }
        let enabled = self
            .persistent_state
            .read()
            .map(|state| state.windows_event_log_diagnostics_enabled)
            .unwrap_or(false);
        if !enabled {
            return false;
        }
        if !self.windows_event_log.is_enabled() {
            self.windows_event_log.enable();
        }
        self.windows_event_log.start_initial_scan()
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn poll_windows_event_log(&mut self) -> bool {
        match self.windows_event_log.poll() {
            WindowsEventLogPoll::Unchanged => false,
            WindowsEventLogPoll::Completed(_) | WindowsEventLogPoll::MarkedIncomplete => true,
        }
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn windows_event_log_worker_poll_interval(&self) -> Option<std::time::Duration> {
        self.windows_event_log.worker_poll_interval()
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn windows_event_log_snapshot(&self) -> WindowsEventLogState {
        self.windows_event_log.ui_snapshot()
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub(crate) fn windows_event_log_diagnostics_enabled(&self) -> bool {
        !self.windows_event_log_disabled_for_session
            && self
                .persistent_state
                .read()
                .map(|state| state.windows_event_log_diagnostics_enabled)
                .unwrap_or(false)
    }

    #[cfg(test)]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn windows_event_log_worker_is_active(&self) -> bool {
        self.windows_event_log.worker_is_active()
    }

    pub fn active_data_dir(&self) -> PathBuf {
        StorageAdapter::active_data_dir()
    }

    pub fn active_storage_mode(&self) -> StateStorageMode {
        StorageAdapter::active_storage_mode()
    }

    pub fn open_active_data_dir(&mut self) {
        let data_dir = self.active_data_dir();
        if let Err(err) = crate::app::adapters::os::open_directory(&data_dir) {
            self.log_manager.add_important_sticky_once(format!(
                "ERROR: Failed to open data folder '{}': {err}",
                data_dir.display()
            ));
        }
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn crash_report_state(&self) -> &CrashReportIndexState {
        self.crash_reports.state()
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn crash_report_directory(&self) -> PathBuf {
        self.crash_reports.report_directory().to_path_buf()
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn request_crash_report_refresh(&mut self) {
        self.crash_reports.request_refresh();
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn poll_crash_report_refresh(&mut self) -> bool {
        let changed = self.crash_reports.poll();
        if changed {
            self.log_manager
                .replace_local_crash_context(self.crash_reports.latest_activity_message());
        }
        changed
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn crash_report_worker_poll_interval(&self) -> Option<std::time::Duration> {
        self.crash_reports.worker_poll_interval()
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn show_crash_report_in_explorer(
        &mut self,
        report: &CrashReportEntry,
    ) -> Result<(), String> {
        let snapshot = self
            .crash_reports
            .state()
            .snapshot()
            .cloned()
            .ok_or_else(|| "the crash report list is not available".to_string())?;
        let path = validated_report_path(&snapshot, report).map_err(|error| error.to_string())?;
        let result = crate::app::adapters::os::show_file_in_directory(&path);
        if result.is_err() {
            self.request_crash_report_refresh();
        }
        result
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn open_crash_report_directory(&mut self) -> Result<(), String> {
        let path = prepare_report_directory(self.crash_reports.report_directory())?;
        crate::app::adapters::os::open_directory_via_shell_broker(&path)
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn delete_crash_report(&mut self, report: &CrashReportEntry) -> Result<(), DeleteError> {
        self.crash_reports.delete_one(report)
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn delete_saved_crash_reports(
        &mut self,
        snapshot: &ReportSnapshot,
    ) -> Result<usize, DeleteError> {
        self.crash_reports.delete_saved_reports_from(snapshot)
    }

    fn filtered_installed_app_entry_indices(&self) -> Vec<usize> {
        let query = self.ui.installed_app_picker.query.trim().to_lowercase();
        let mut matches: Vec<(usize, (usize, usize, String, String))> = self
            .ui
            .installed_app_picker
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                Self::installed_app_query_sort_key(entry, &query).map(|key| (index, key))
            })
            .collect();

        matches.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
        matches.into_iter().map(|(index, _)| index).collect()
    }

    fn installed_app_query_sort_key(
        entry: &InstalledAppCatalogEntry,
        query: &str,
    ) -> Option<(usize, usize, String, String)> {
        let name = entry.name.to_lowercase();
        let detail = entry.detail.to_lowercase();

        if query.is_empty() {
            if entry.source.hide_until_query() {
                return None;
            }

            return Some((0, entry.source.picker_priority(), name, detail));
        }

        let match_rank = if name == query {
            0
        } else if name.starts_with(query) {
            1
        } else if name.contains(query) {
            2
        } else if detail.starts_with(query) {
            3
        } else if detail.contains(query) {
            4
        } else {
            return None;
        };

        Some((match_rank, entry.source.picker_priority(), name, detail))
    }

    fn normalize_installed_app_picker_selection(&mut self) {
        let filtered = self.filtered_installed_app_entry_indices();
        let picker = &mut self.ui.installed_app_picker;

        if filtered.is_empty() {
            picker.selected_entry_index = None;
            return;
        }

        if picker
            .selected_entry_index
            .is_some_and(|index| filtered.contains(&index))
        {
            return;
        }

        picker.selected_entry_index = Some(filtered[0]);
    }

    fn reset_installed_app_picker_session(&mut self) {
        let picker = &mut self.ui.installed_app_picker;
        picker.target_group_id = None;
        picker.query.clear();
        picker.selected_entry_index = None;
        picker.is_refreshing = false;
        picker.last_error = None;
        picker.needs_focus = false;
        picker.refresh_rx = None;
    }

    fn reset_app_run_settings_session(&mut self) {
        self.ui.app_edit_state.current_edit = None;
        self.ui.app_edit_state.target = None;
        self.ui.app_edit_state.shortcut_result = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn followup_classic_save_reports_acceptance_before_gated_completion() {
        use crate::app::features::system_tweaks::{
            service::{SavedState, StatePersistence},
            SaveError, TuningOperationState, TuningRuntime,
        };
        use std::sync::mpsc;
        struct GatedSave {
            entered: mpsc::SyncSender<()>,
            release: mpsc::Receiver<()>,
            fail: bool,
        }
        impl StatePersistence for GatedSave {
            fn save(&mut self, _: &mut AppStateStorage) -> Result<SavedState, SaveError> {
                self.entered.send(()).unwrap();
                if self
                    .release
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .is_err()
                    || self.fail
                {
                    Err(SaveError::Storage)
                } else {
                    Ok(SavedState::Unavailable)
                }
            }
        }
        for fail in [false, true] {
            let mut app = sample_state();
            let original = app.persistent_state.read().unwrap().clone();
            let (entered_tx, entered_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let runtime = TuningRuntime::start(
                crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                    .headless(
                        original.clone(),
                        Box::new(GatedSave {
                            entered: entered_tx,
                            release: release_rx,
                            fail,
                        }),
                    )
                    .without_modules(),
                app.runtime.running_apps_handle(),
            );
            app.replace_tuning_for_test(runtime, Default::default());
            let mut candidate = original.clone();
            candidate.theme_index = 2;
            let accepted = app.enqueue_legacy(
                super::TuningRequest::SaveBase {
                    candidate: Box::new(candidate),
                    expected_revision: 0,
                },
                None,
            );
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            let pending = matches!(
                app.tuning_snapshot().operation,
                TuningOperationState::Pending { .. }
            );
            let before = app.persistent_state.read().unwrap().theme_index;
            let _ = release_tx.send(());
            assert!(
                accepted,
                "acceptance is independent of persistence completion"
            );
            assert!(
                pending,
                "the gated save must still be pending after enqueue returns"
            );
            assert_eq!(before, original.theme_index);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while matches!(
                app.tuning_snapshot().operation,
                TuningOperationState::Pending { .. }
            ) {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            app.poll_tuning();
            assert_eq!(
                app.get_theme_index(),
                if fail { original.theme_index } else { 2 }
            );
            assert!(app.pending_legacy.is_none());
            assert_eq!(
                matches!(
                    app.tuning_snapshot().operation,
                    TuningOperationState::Refused {
                        error: SaveError::Storage,
                        ..
                    }
                ),
                fail
            );
        }
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn cpu_load_retention_save_preserves_existing_candidate_and_other_preferences() {
        let mut app = sample_state();
        let classic = app.is_process_monitoring_enabled();
        let theme = app.get_theme_index();
        assert_eq!(app.cpu_load_history_seconds(), 300);
        app.set_cpu_load_history_seconds(30).unwrap();
        app.wait_for_test_save();
        assert_eq!(app.cpu_load_history_seconds(), 30);
        assert_eq!(
            app.persistent_state
                .read()
                .unwrap()
                .cpu_load_history_seconds,
            30
        );
        app.pending_legacy = Some(super::PendingLegacy {
            id: 12345,
            revision: 77,
            completion: Some(super::LegacyCompletion::Group(
                None,
                "Keep draft".into(),
                vec![true],
                false,
            )),
        });
        assert!(app.set_cpu_load_history_seconds(1800).is_err());
        let pending = app.pending_legacy.as_ref().unwrap();
        assert_eq!((pending.id, pending.revision), (12345, 77));
        assert!(
            matches!(&pending.completion, Some(super::LegacyCompletion::Group(None, name, _, false)) if name == "Keep draft")
        );
        assert_eq!(app.cpu_load_history_seconds(), 30);
        assert_eq!(
            app.persistent_state
                .read()
                .unwrap()
                .cpu_load_history_seconds,
            30
        );
        app.pending_legacy = None;
        assert!(app.set_cpu_load_history_seconds(60).is_err());
        app.set_cpu_load_history_seconds(1800).unwrap();
        app.wait_for_test_save();
        let reloaded: AppStateStorage = serde_json::from_str(
            &serde_json::to_string(&*app.persistent_state.read().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(reloaded.cpu_load_history_seconds, 1800);
        assert_eq!(app.is_process_monitoring_enabled(), classic);
        assert_eq!(app.get_theme_index(), theme);
    }
    #[cfg(not(all(target_os = "windows", feature = "windows")))]
    #[test]
    fn linux_modules_route_is_unavailable() {
        let mut app = sample_state();
        app.set_current_window(WindowRoute::Tuning);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
    }

    #[cfg(not(all(target_os = "windows", feature = "windows")))]
    #[test]
    fn linux_modules_requests_are_refused_without_idle_module_polling() {
        let mut app = sample_state();
        assert_eq!(
            app.request_tuning(
                crate::app::features::system_tweaks::TuningRequest::CreateProfile {
                    name: "Unavailable".into(),
                }
            ),
            Err(crate::app::features::system_tweaks::SaveError::Invalid)
        );
        assert_eq!(app.tuning_poll_interval(), None);
        assert!(app.tuning_snapshot().catalog.is_empty());
        assert!(app.tuning_snapshot().profiles.is_empty());
    }

    #[cfg(not(all(target_os = "windows", feature = "windows")))]
    #[test]
    fn linux_modules_disabled_preserves_raw_data_during_classic_rule_changes() {
        let original = sample_state();
        let mut saved = original.persistent_state.read().unwrap().clone();
        saved.version = 11;
        saved.loaded_version = 11;
        let raw = r#"{ "version": 77, "future": { "preserve": [1, 2] } }"#;
        saved.tuning = serde_json::from_str(raw).unwrap();
        let mut app = AppState::new_for_test(Arc::new(RwLock::new(saved)), 2);
        let group = app.rules.group_id_for_index(0).unwrap();
        let rule = app.rules.rule_id_for_index(0, 0).unwrap();
        assert!(app.enqueue_legacy(
            crate::app::features::system_tweaks::TuningRequest::RemoveRule { group, rule },
            None,
        ));
        app.wait_for_test_save();
        assert!(app.persistent_state.read().unwrap().groups[0]
            .programs
            .is_empty());
        app.set_theme_index(2);
        app.wait_for_test_save();
        assert_eq!(app.get_theme_index(), 2);
        assert_eq!(app.persistent_state.read().unwrap().tuning.raw(), Some(raw));
    }

    #[test]
    fn p4_legacy_save_failure_keeps_saved_values_ids_and_rule_editor() {
        use crate::app::features::system_tweaks::{
            service::{SavedState, StatePersistence},
            SaveError, TuningOperationState, TuningRuntime,
        };
        struct Failing;
        impl StatePersistence for Failing {
            fn save(&mut self, _: &mut AppStateStorage) -> Result<SavedState, SaveError> {
                Err(SaveError::Storage)
            }
        }
        let mut app = sample_state();
        let state = app.persistent_state.read().unwrap().clone();
        let runtime = TuningRuntime::start(
            crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                .headless(state, Box::new(Failing)),
            app.runtime.running_apps_handle(),
        );
        app.replace_tuning_for_test(runtime, Default::default());
        let group = app.rules.group_id_for_index(0).unwrap();
        let rule = app.rules.rule_id_for_index(0, 0).unwrap();
        let identities = app.rules.to_persisted_identities();
        app.open_app_run_settings(group, rule);
        app.ensure_current_edit_loaded();
        app.ui.app_edit_state.current_edit.as_mut().unwrap().name = "Unsaved draft".into();
        app.commit_current_app_edit_session();
        app.wait_for_test_save();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while matches!(
            app.tuning_snapshot().operation,
            TuningOperationState::Pending { .. }
        ) && std::time::Instant::now() < deadline
        {
            app.poll_tuning();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.poll_tuning();
        assert_ne!(app.get_group_program(0, 0).unwrap().name, "Unsaved draft");
        assert_eq!(app.rules.to_persisted_identities(), identities);
        assert_eq!(
            app.ui.app_edit_state.current_edit.as_ref().unwrap().name,
            "Unsaved draft"
        );
        assert!(matches!(app.ui.current_window, WindowRoute::AppRunSettings));
        app.set_theme_index(2);
        app.wait_for_test_save();
        assert_eq!(app.get_theme_index(), 0);
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use super::RuleShortcutDisabledReason;
    use super::{AppState, MoveRuleToGroupOutcome, RunRuleOutcome, TrackedProcessSnapshot};
    use crate::app::features::rules::RulesContext;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use crate::app::features::shortcut::{
        CreateRuleShortcutError, RuleShortcutPlatform, ShortcutWriteError,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, LaunchTarget};
    use crate::app::shared::ids::{GroupId, RuleId};
    use crate::app::shell::sessions::{RuleEditorTarget, RuleShortcutResult, ShortcutCreationRole};
    use crate::app::shell::{GroupRoute, WindowRoute};
    use os_api::PriorityClass;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use os_api::ShortcutSpec;
    use os_api::{InstalledAppCatalogEntry, InstalledAppCatalogSource};
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use std::collections::HashSet;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    #[cfg(all(target_os = "windows", feature = "windows"))]
    struct FakeShortcutPlatform {
        supported: bool,
        current_exe: Result<PathBuf, String>,
        desktop_dir: Result<PathBuf, String>,
        existing_paths: HashSet<String>,
        create_calls: Vec<ShortcutSpec>,
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    impl FakeShortcutPlatform {
        fn supported() -> Self {
            Self {
                supported: true,
                current_exe: Ok(PathBuf::from(r"C:\Tools\cpu-affinity-tool.exe")),
                desktop_dir: Ok(PathBuf::from(r"C:\Users\Ada\Desktop")),
                existing_paths: HashSet::new(),
                create_calls: Vec::new(),
            }
        }

        fn path_key(path: &Path) -> String {
            path.to_string_lossy().to_ascii_lowercase()
        }
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    impl RuleShortcutPlatform for FakeShortcutPlatform {
        fn is_supported(&self) -> bool {
            self.supported
        }

        fn current_exe_path(&mut self) -> Result<PathBuf, String> {
            self.current_exe.clone()
        }

        fn current_user_desktop_dir(&mut self) -> Result<PathBuf, String> {
            self.desktop_dir.clone()
        }

        fn shortcut_path_exists(&mut self, path: &Path) -> Result<bool, String> {
            Ok(self.existing_paths.contains(&Self::path_key(path)))
        }

        fn create_shortcut_new(&mut self, spec: ShortcutSpec) -> Result<(), ShortcutWriteError> {
            self.create_calls.push(spec);
            Ok(())
        }
    }

    fn sample_state() -> AppState {
        let persistent_state = Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
            state.version = 5;
            state.groups = vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![0, 1],
                programs: vec![AppToRun::new_path(
                    PathBuf::from(r"C:\Sample.lnk"),
                    vec![],
                    PathBuf::from(r"C:\Sample.exe"),
                    PriorityClass::Normal,
                    false,
                )],
                is_hidden: false,
                run_all_button: true,
            }];
            state.cpu_schema = CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            };
            state.theme_index = 0;
            state.process_monitoring_enabled = false;
            state.windows_event_log_diagnostics_enabled = true;
            state.start_minimized = false;
            state.rule_identities = None;
            state.loaded_version = 5;
            state.pending_pre_v6_backup = false;
            state.tuning = Default::default();
        })));

        let rules = persistent_state
            .read()
            .map(|state| RulesContext::from_storage(&state))
            .unwrap();

        let _ = rules;
        AppState::new_for_test(persistent_state, 4)
    }

    fn group_id(app: &AppState, index: usize) -> GroupId {
        app.rules.group_id_for_index(index).unwrap()
    }

    fn rule_id(app: &AppState, group_index: usize, rule_index: usize) -> RuleId {
        app.rules
            .rule_id_for_index(group_index, rule_index)
            .unwrap()
    }

    fn add_empty_group(app: &mut AppState, name: &str) {
        app.persistent_state
            .write()
            .unwrap()
            .groups
            .push(CoreGroup {
                name: name.to_string(),
                cores: vec![2, 3],
                programs: Vec::new(),
                is_hidden: false,
                run_all_button: false,
            });
        app.reconcile_rules();
        rebase_sample_fixture(app);
    }
    fn rebase_sample_fixture(app: &mut AppState) {
        let mut saved = app.persistent_state.read().unwrap().clone();
        saved.mark_ready_for_current_schema_save(app.rules.to_persisted_identities());
        let runtime = crate::app::features::system_tweaks::TuningRuntime::start(
            crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                .headless(saved, Box::new(super::TestStatePersistence)),
            app.runtime.running_apps_handle(),
        );
        app.replace_tuning_for_test(runtime, Default::default());
    }

    fn sample_app(name: &str) -> AppToRun {
        let mut app = AppToRun::new_path(
            PathBuf::from(format!(r"C:\{name}.lnk")),
            vec![],
            PathBuf::from(format!(r"C:\{name}.exe")),
            PriorityClass::Normal,
            false,
        );
        app.name = name.to_string();
        app
    }

    fn edit_first_rule(app: &mut AppState, updated: AppToRun) {
        app.ui.current_window = WindowRoute::AppRunSettings;
        app.ui.app_edit_state.target = Some(RuleEditorTarget {
            group_id: group_id(app, 0),
            rule_id: rule_id(app, 0, 0),
        });
        app.ui.app_edit_state.current_edit = Some(updated);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_current_app_edit_shortcut_status_enabled_for_clean_saved_rule() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.set_shortcut_creation_role(ShortcutCreationRole::Primary);

        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());

        let status = app.current_app_edit_shortcut_status();
        assert!(status.visible);
        assert!(status.enabled);
        assert_eq!(status.disabled_reason, None);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_current_app_edit_shortcut_status_disabled_for_non_primary_instance() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.set_shortcut_creation_role(ShortcutCreationRole::NonPrimary);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());

        let status = app.current_app_edit_shortcut_status();

        assert!(status.visible);
        assert!(!status.enabled);
        assert_eq!(
            status.disabled_reason,
            Some(RuleShortcutDisabledReason::NonPrimary)
        );
        assert_eq!(
            status.message.as_deref(),
            Some("Close the other running instance first.")
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_current_app_edit_shortcut_status_requires_saved_changes_first() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());
        app.ui.app_edit_state.current_edit.as_mut().unwrap().name = "Unsaved".to_string();

        let status = app.current_app_edit_shortcut_status();

        assert!(status.visible);
        assert!(!status.enabled);
        assert_eq!(
            status.disabled_reason,
            Some(RuleShortcutDisabledReason::SaveChangesFirst)
        );
    }

    #[cfg(feature = "linux")]
    #[test]
    fn test_current_app_edit_shortcut_status_hidden_for_linux_feature() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());

        let status = app.current_app_edit_shortcut_status();

        assert!(!status.visible);
        assert!(!status.enabled);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_create_shortcut_for_current_rule_uses_saved_rule_and_stores_success() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());
        let mut platform = FakeShortcutPlatform::supported();

        let created = app
            .create_shortcut_for_current_rule_with_platform(&mut platform)
            .unwrap();

        assert_eq!(
            created,
            PathBuf::from(r"C:\Users\Ada\Desktop\Sample - Games.lnk")
        );
        assert_eq!(platform.create_calls.len(), 1);
        assert_eq!(
            app.ui.app_edit_state.shortcut_result,
            Some(RuleShortcutResult::Created {
                filename: "Sample - Games.lnk".to_string()
            })
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_create_shortcut_for_current_rule_dirty_draft_does_not_call_platform() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());
        app.ui.app_edit_state.current_edit.as_mut().unwrap().name = "Unsaved".to_string();
        let mut platform = FakeShortcutPlatform::supported();

        assert_eq!(
            app.create_shortcut_for_current_rule_with_platform(&mut platform),
            Err(CreateRuleShortcutError::SaveChangesFirst)
        );

        assert!(platform.create_calls.is_empty());
        assert_eq!(
            app.ui.app_edit_state.shortcut_result,
            Some(RuleShortcutResult::Failed {
                message: "Save changes before creating a shortcut.".to_string()
            })
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_create_shortcut_for_current_rule_non_primary_does_not_call_platform() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.set_shortcut_creation_role(ShortcutCreationRole::NonPrimary);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());
        let mut platform = FakeShortcutPlatform::supported();

        assert_eq!(
            app.create_shortcut_for_current_rule_with_platform(&mut platform),
            Err(CreateRuleShortcutError::UnsupportedPlatform)
        );

        assert!(platform.create_calls.is_empty());
        assert_eq!(
            app.ui.app_edit_state.shortcut_result,
            Some(RuleShortcutResult::Failed {
                message: "Close the other running instance first.".to_string()
            })
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn test_create_shortcut_for_current_rule_rechecks_after_same_frame_dirty_change() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);
        let rule_id = rule_id(&app, 0, 0);
        app.open_app_run_settings(group_id, rule_id);
        assert!(app.ensure_current_edit_loaded());
        let initial_status = app.current_app_edit_shortcut_status();
        assert!(initial_status.enabled);
        app.ui.app_edit_state.current_edit.as_mut().unwrap().name = "Unsaved".to_string();
        let mut platform = FakeShortcutPlatform::supported();

        assert_eq!(
            app.create_shortcut_for_current_rule_with_platform(&mut platform),
            Err(CreateRuleShortcutError::SaveChangesFirst)
        );

        assert!(platform.create_calls.is_empty());
    }

    #[test]
    fn test_run_group_program_reports_missing_group() {
        let mut app = sample_state();
        let existing_rule_id = rule_id(&app, 0, 0);

        assert_eq!(
            app.run_group_program(GroupId("missing-group".to_string()), existing_rule_id),
            RunRuleOutcome::MissingGroup
        );
    }

    #[test]
    fn test_run_group_program_reports_missing_rule() {
        let mut app = sample_state();
        let existing_group_id = group_id(&app, 0);

        assert_eq!(
            app.run_group_program(existing_group_id, RuleId("missing-rule".to_string())),
            RunRuleOutcome::MissingRule
        );
    }

    #[test]
    fn test_run_group_program_rejects_rule_moved_to_another_group() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Background");
        let source_group_id = group_id(&app, 0);
        let target_group_id = group_id(&app, 1);
        let moved_rule_id = rule_id(&app, 0, 0);

        assert_eq!(
            app.move_rule_to_group_at(
                source_group_id.clone(),
                moved_rule_id.clone(),
                target_group_id,
                0
            ),
            MoveRuleToGroupOutcome::Moved
        );
        app.wait_for_test_save();

        assert_eq!(
            app.run_group_program(source_group_id, moved_rule_id),
            RunRuleOutcome::MissingRule
        );
    }

    #[test]
    fn test_run_group_program_rejects_existing_rule_without_verified_identity() {
        let mut app = sample_state();
        let existing_group_id = group_id(&app, 0);
        let existing_rule_id = rule_id(&app, 0, 0);
        let app_key = app.get_group_program(0, 0).unwrap().get_key();

        assert!(app.runtime.add_running_app(
            &app_key,
            12345,
            existing_group_id.clone(),
            existing_rule_id.clone()
        ));

        assert!(matches!(
            app.run_group_program(existing_group_id, existing_rule_id),
            RunRuleOutcome::LaunchRejected(message)
                if message.contains("tracked process identity is not available")
        ));
    }

    #[test]
    fn test_run_group_program_reports_launch_rejection() {
        let mut app = sample_state();
        let existing_group_id = group_id(&app, 0);
        let existing_rule_id = rule_id(&app, 0, 0);

        let outcome = app.run_group_program(existing_group_id, existing_rule_id);

        assert!(matches!(
            outcome,
            RunRuleOutcome::LaunchRejected(message)
                if message.contains("Failed to start process")
                    || message.contains("No such file")
                    || message.contains("The system cannot find")
        ));
    }

    #[test]
    fn test_commit_group_form_session_keeps_invalid_create_draft_visible() {
        let mut app = sample_state();
        app.ui.current_window = WindowRoute::Groups(GroupRoute::Create);

        app.commit_group_form_session();
        app.wait_for_test_save();

        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::Create)
        ));
        assert!(app.ui.group_form.group_name.is_empty());
        assert!(app
            .ui
            .group_form
            .core_selection
            .iter()
            .all(|selected| !selected));
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 1);
        assert_eq!(app.save_count(), 0);
        assert_eq!(app.log_manager.entries.len(), 1);
        assert_eq!(
            app.log_manager.entries[0].message,
            "Group name cannot be empty"
        );
    }

    #[test]
    fn test_start_creating_group_clears_previous_edit_session() {
        let mut app = sample_state();
        let existing_group_id = group_id(&app, 0);
        app.start_editing_group(existing_group_id);

        assert!(app.ui.group_form.editing_group_id.is_some());
        assert!(!app.ui.group_form.group_name.is_empty());

        app.start_creating_group();

        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::Create)
        ));
        assert!(app.ui.group_form.editing_group_id.is_none());
        assert!(app.ui.group_form.group_name.is_empty());
        assert!(app
            .ui
            .group_form
            .core_selection
            .iter()
            .all(|selected| !selected));
    }

    #[test]
    fn test_commit_current_app_edit_session_updates_and_closes() {
        let mut app = sample_state();
        let mut updated = AppToRun::new_path(
            PathBuf::from(r"C:\Sample.lnk"),
            vec!["--debug".to_string()],
            PathBuf::from(r"C:\Updated.exe"),
            PriorityClass::High,
            true,
        );
        updated.name = "Updated".to_string();
        updated.additional_processes = vec!["helper.exe".to_string()];
        edit_first_rule(&mut app, updated);

        app.commit_current_app_edit_session();
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        let updated = &state.groups[0].programs[0];
        assert_eq!(updated.name, "Updated");
        assert_eq!(
            updated.bin_path(),
            Some(PathBuf::from(r"C:\Updated.exe").as_path())
        );
        drop(state);
        assert_eq!(app.save_count(), 1);

        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.app_edit_state.current_edit.is_none());
        assert!(app.ui.app_edit_state.target.is_none());
    }

    #[test]
    fn test_commit_current_app_edit_session_replaces_primary_tracked_name() {
        let mut app = sample_state();
        let mut updated = app.persistent_state.read().unwrap().groups[0].programs[0].clone();
        *updated.bin_path_mut().unwrap() = PathBuf::from(r"C:\Updated.exe");
        updated.additional_processes = vec!["Sample.exe".to_string(), "helper.exe".to_string()];
        edit_first_rule(&mut app, updated);

        app.commit_current_app_edit_session();
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        assert_eq!(
            state.groups[0].programs[0].additional_processes,
            vec!["Updated.exe".to_string(), "helper.exe".to_string()]
        );
        drop(state);
        assert_eq!(app.save_count(), 1);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.app_edit_state.current_edit.is_none());
        assert!(app.ui.app_edit_state.target.is_none());
    }

    #[test]
    fn test_commit_current_app_edit_session_respects_removed_primary_tracked_name() {
        let mut app = sample_state();
        let mut updated = app.persistent_state.read().unwrap().groups[0].programs[0].clone();
        *updated.bin_path_mut().unwrap() = PathBuf::from(r"C:\Updated.exe");
        updated.additional_processes.clear();
        edit_first_rule(&mut app, updated);

        app.commit_current_app_edit_session();
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        assert!(state.groups[0].programs[0].additional_processes.is_empty());
        drop(state);
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_commit_current_app_edit_session_removes_old_primary_when_new_primary_exists() {
        let mut app = sample_state();
        let mut updated = app.persistent_state.read().unwrap().groups[0].programs[0].clone();
        *updated.bin_path_mut().unwrap() = PathBuf::from(r"C:\Updated.exe");
        updated.additional_processes = vec![
            "Sample.exe".to_string(),
            "Updated.exe".to_string(),
            "helper.exe".to_string(),
        ];
        edit_first_rule(&mut app, updated);

        app.commit_current_app_edit_session();
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        assert_eq!(
            state.groups[0].programs[0].additional_processes,
            vec!["Updated.exe".to_string(), "helper.exe".to_string()]
        );
        drop(state);
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_move_rule_to_group_preserves_app_and_rule_id() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Background");
        let source_group_id = group_id(&app, 0);
        let target_group_id = group_id(&app, 1);
        let moved_rule_id = rule_id(&app, 0, 0);
        let original_app = app.persistent_state.read().unwrap().groups[0].programs[0].clone();

        let outcome =
            app.move_rule_to_group_at(source_group_id, moved_rule_id.clone(), target_group_id, 0);
        app.wait_for_test_save();

        assert_eq!(outcome, MoveRuleToGroupOutcome::Moved);
        let state = app.persistent_state.read().unwrap();
        assert!(state.groups[0].programs.is_empty());
        assert_eq!(state.groups[1].programs, vec![original_app]);
        drop(state);
        assert_eq!(app.rules.rule_id_for_index(1, 0), Some(moved_rule_id));
        assert!(app.rules.rule_id_for_index(0, 0).is_none());
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_move_rule_to_group_at_inserts_at_target_position() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Background");
        app.persistent_state.write().unwrap().groups[1].programs =
            vec![sample_app("TargetFirst"), sample_app("TargetSecond")];
        app.reconcile_rules();
        rebase_sample_fixture(&mut app);

        let source_group_id = group_id(&app, 0);
        let target_group_id = group_id(&app, 1);
        let moved_rule_id = rule_id(&app, 0, 0);
        let original_app = app.persistent_state.read().unwrap().groups[0].programs[0].clone();

        let outcome =
            app.move_rule_to_group_at(source_group_id, moved_rule_id.clone(), target_group_id, 1);
        app.wait_for_test_save();

        assert_eq!(outcome, MoveRuleToGroupOutcome::Moved);
        let state = app.persistent_state.read().unwrap();
        assert!(state.groups[0].programs.is_empty());
        assert_eq!(state.groups[1].programs[0].name, "TargetFirst");
        assert_eq!(state.groups[1].programs[1], original_app);
        assert_eq!(state.groups[1].programs[2].name, "TargetSecond");
        drop(state);
        assert_eq!(app.rules.rule_id_for_index(1, 1), Some(moved_rule_id));
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_reorder_rule_within_group_preserves_app_and_rule_id() {
        let mut app = sample_state();
        app.persistent_state.write().unwrap().groups[0]
            .programs
            .extend([sample_app("Second"), sample_app("Third")]);
        app.reconcile_rules();
        rebase_sample_fixture(&mut app);
        let group_id = group_id(&app, 0);
        let first_rule_id = rule_id(&app, 0, 0);
        let second_rule_id = rule_id(&app, 0, 1);
        let third_rule_id = rule_id(&app, 0, 2);

        let outcome =
            app.move_rule_to_group_at(group_id.clone(), first_rule_id.clone(), group_id, 3);
        app.wait_for_test_save();

        assert_eq!(outcome, MoveRuleToGroupOutcome::Moved);
        let state = app.persistent_state.read().unwrap();
        let names = state.groups[0]
            .programs
            .iter()
            .map(|program| program.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Second", "Third", "Sample"]);
        drop(state);
        assert_eq!(app.rules.rule_id_for_index(0, 0), Some(second_rule_id));
        assert_eq!(app.rules.rule_id_for_index(0, 1), Some(third_rule_id));
        assert_eq!(app.rules.rule_id_for_index(0, 2), Some(first_rule_id));
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_move_rule_to_same_group_and_duplicate_target_do_not_save() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Background");
        let source_group_id = group_id(&app, 0);
        let target_group_id = group_id(&app, 1);
        let moved_rule_id = rule_id(&app, 0, 0);

        assert_eq!(
            app.move_rule_to_group_at(
                source_group_id.clone(),
                moved_rule_id.clone(),
                source_group_id.clone(),
                0,
            ),
            MoveRuleToGroupOutcome::SamePosition
        );
        app.wait_for_test_save();
        assert_eq!(app.save_count(), 0);

        let duplicate = app.persistent_state.read().unwrap().groups[0].programs[0].clone();
        app.persistent_state.write().unwrap().groups[1]
            .programs
            .push(duplicate);
        app.reconcile_rules();

        assert_eq!(
            app.move_rule_to_group_at(source_group_id, moved_rule_id, target_group_id, 0),
            MoveRuleToGroupOutcome::DuplicateInTarget
        );
        app.wait_for_test_save();
        let state = app.persistent_state.read().unwrap();
        assert_eq!(state.groups[0].programs.len(), 1);
        assert_eq!(state.groups[1].programs.len(), 1);
        drop(state);
        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_select_theme_saves_only_valid_changes_and_monitoring_still_saves() {
        let mut app = sample_state();

        app.set_theme_index(2);
        app.wait_for_test_save();
        assert_eq!(app.get_theme_index(), 2);
        assert_eq!(app.save_count(), 1);
        app.set_theme_index(2);
        app.wait_for_test_save();
        app.set_theme_index(99);
        app.wait_for_test_save();
        assert_eq!(app.get_theme_index(), 2);
        assert_eq!(app.save_count(), 1);
        app.set_theme_index(0);
        app.wait_for_test_save();
        assert_eq!(app.get_theme_index(), 0);
        assert_eq!(app.save_count(), 2);

        app.toggle_process_monitoring();
        app.wait_for_test_save();
        assert!(app.is_process_monitoring_enabled());
        assert_eq!(app.save_count(), 3);
    }

    #[test]
    fn start_minimized_is_saved_and_missing_preference_defaults_to_visible() {
        let mut app = sample_state();
        assert!(!app.start_minimized());
        app.set_start_minimized(true).unwrap();
        app.wait_for_test_save();
        assert!(app.start_minimized());
        assert_eq!(app.save_count(), 1);
        app.set_start_minimized(true).unwrap();
        app.wait_for_test_save();
        assert_eq!(app.save_count(), 1);
        let json = serde_json::to_string(&*app.persistent_state.read().unwrap()).unwrap();
        let restored: AppStateStorage = serde_json::from_str(&json).unwrap();
        assert!(restored.start_minimized);
        let mut old: serde_json::Value = serde_json::from_str(&json).unwrap();
        old.as_object_mut().unwrap().remove("start_minimized");
        assert!(
            !serde_json::from_value::<AppStateStorage>(old)
                .unwrap()
                .start_minimized
        );
        app.set_start_minimized(false).unwrap();
        app.wait_for_test_save();
        assert!(!app.start_minimized());
        assert_eq!(app.save_count(), 2);
    }

    #[test]
    fn test_set_group_hidden_saves_only_on_real_change() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);

        app.set_group_is_hidden(group_id.clone(), true);
        app.wait_for_test_save();
        assert!(app.persistent_state.read().unwrap().groups[0].is_hidden);
        assert_eq!(app.save_count(), 1);

        app.set_group_is_hidden(group_id, true);
        app.wait_for_test_save();
        assert_eq!(app.save_count(), 1);

        app.set_group_is_hidden(GroupId("missing-group".to_string()), false);
        app.wait_for_test_save();
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_move_group_invalid_target_or_missing_group_do_not_save() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Work");
        let first_group_id = group_id(&app, 0);

        assert!(!app.move_group_to_index(first_group_id, 0));
        app.wait_for_test_save();
        assert!(!app.move_group_to_index(GroupId("missing-group".to_string()), 1));
        app.wait_for_test_save();
        assert!(!app.move_group_to_index(group_id(&app, 0), 99));
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        assert_eq!(state.groups[0].name, "Games");
        assert_eq!(state.groups[1].name, "Work");
        drop(state);
        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_move_group_to_index_preserves_ids_and_saves_once() {
        let mut app = sample_state();
        let second = CoreGroup {
            name: "Second".to_string(),
            cores: vec![2],
            programs: Vec::new(),
            is_hidden: false,
            run_all_button: false,
        };
        let third = CoreGroup {
            name: "Third".to_string(),
            cores: vec![3],
            programs: Vec::new(),
            is_hidden: false,
            run_all_button: false,
        };
        app.persistent_state
            .write()
            .unwrap()
            .groups
            .extend([second, third]);
        app.reconcile_rules();
        let original_ids = (0..3)
            .map(|index| app.rules.group_id_for_index(index).unwrap())
            .collect::<Vec<_>>();

        assert!(app.move_group_to_index(original_ids[0].clone(), 2));
        app.wait_for_test_save();

        let names = app
            .persistent_state
            .read()
            .unwrap()
            .groups
            .iter()
            .map(|group| group.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Second", "Third", "Games"]);
        assert_eq!(
            app.rules.group_id_for_index(0),
            Some(original_ids[1].clone())
        );
        assert_eq!(
            app.rules.group_id_for_index(1),
            Some(original_ids[2].clone())
        );
        assert_eq!(
            app.rules.group_id_for_index(2),
            Some(original_ids[0].clone())
        );
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_successful_group_create_and_delete_save_once_each() {
        let mut app = sample_state();
        app.ui.group_form.group_name = "Work".to_string();
        app.ui.group_form.core_selection[2] = true;

        app.commit_group_form_session();
        app.wait_for_test_save();
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 2);
        assert_eq!(app.save_count(), 1);

        app.ui.group_form.editing_group_id = Some(group_id(&app, 1));
        app.delete_current_group_form_target();
        app.wait_for_test_save();
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 1);
        assert_eq!(app.save_count(), 2);
    }

    #[test]
    fn test_delete_current_app_edit_target_saves_once() {
        let mut app = sample_state();
        app.ui.app_edit_state.target = Some(RuleEditorTarget {
            group_id: group_id(&app, 0),
            rule_id: rule_id(&app, 0, 0),
        });

        app.delete_current_app_edit_target();
        app.wait_for_test_save();

        assert!(app.persistent_state.read().unwrap().groups[0]
            .programs
            .is_empty());
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_noop_delete_current_app_edit_target_does_not_save() {
        let mut app = sample_state();

        app.delete_current_app_edit_target();
        app.wait_for_test_save();

        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_consume_dropped_files_without_pending_files_clears_cached_target() {
        let mut app = sample_state();
        let target_group_id = group_id(&app, 0);
        app.ui.file_drop_hover_target = Some(target_group_id.clone());

        assert!(!app.consume_dropped_files_into_group(target_group_id));
        app.wait_for_test_save();

        assert!(app.ui.file_drop_hover_target.is_none());
        assert!(app.ui.dropped_files.is_none());
        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_consume_dropped_files_with_stale_group_clears_pending_files_without_save() {
        let mut app = sample_state();
        app.ui.dropped_files = Some(vec![PathBuf::from(r"C:\Dropped.exe")]);
        app.ui.file_drop_hover_target = Some(GroupId("stale-group".to_string()));

        assert!(!app.consume_dropped_files_into_group(GroupId("stale-group".to_string())));
        app.wait_for_test_save();

        assert!(app.ui.file_drop_hover_target.is_none());
        assert!(app.ui.dropped_files.is_none());
        assert_eq!(
            app.persistent_state.read().unwrap().groups[0]
                .programs
                .len(),
            1
        );
        assert_eq!(app.save_count(), 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_consume_dropped_files_adds_windows_exe_to_target_and_clears_transient_state() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Dropped");
        let target_group_id = group_id(&app, 1);
        app.ui.dropped_files = Some(vec![PathBuf::from(r"C:\Dropped.exe")]);
        app.ui.file_drop_hover_target = Some(target_group_id.clone());

        assert!(app.consume_dropped_files_into_group(target_group_id));
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        assert_eq!(state.groups[0].programs.len(), 1);
        assert_eq!(state.groups[1].programs.len(), 1);
        assert_eq!(
            state.groups[1].programs[0].bin_path(),
            Some(PathBuf::from(r"C:\Dropped.exe").as_path())
        );
        drop(state);
        assert!(app.ui.file_drop_hover_target.is_none());
        assert!(app.ui.dropped_files.is_none());
        assert!(app.rules.rule_id_for_index(1, 0).is_some());
        assert_eq!(app.save_count(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_add_selected_files_partial_success_saves_once() {
        let mut app = sample_state();

        app.add_selected_files_to_group(
            group_id(&app, 0),
            vec![r"C:\valid.exe".into(), r"C:\broken".into()],
        );
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        assert_eq!(state.groups[0].programs.len(), 2);
        assert_eq!(
            state.groups[0].programs[1].bin_path(),
            Some(PathBuf::from(r"C:\valid.exe").as_path())
        );
        drop(state);
        assert_eq!(app.save_count(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_add_selected_files_all_invalid_does_not_save() {
        let mut app = sample_state();

        app.add_selected_files_to_group(group_id(&app, 0), vec![r"C:\broken".into()]);
        app.wait_for_test_save();

        assert_eq!(
            app.persistent_state.read().unwrap().groups[0]
                .programs
                .len(),
            1
        );
        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_central_snapshot_preserves_logical_ids() {
        let mut app = sample_state();
        app.persistent_state
            .write()
            .unwrap()
            .groups
            .push(CoreGroup {
                name: "Work".to_string(),
                cores: vec![2, 3],
                programs: vec![],
                is_hidden: true,
                run_all_button: false,
            });

        let snapshot = app.build_central_panel_snapshot();

        assert_eq!(snapshot.groups.len(), 2);
        assert_eq!(snapshot.groups[0].group_id, group_id(&app, 0));
        assert_eq!(snapshot.groups[0].programs[0].rule_id, rule_id(&app, 0, 0));
        assert_eq!(snapshot.groups[1].group_id, group_id(&app, 1));
        assert!(snapshot.groups[1].is_hidden);
    }

    #[test]
    fn tracked_process_refresh_keeps_last_rows_on_busy_and_limits_inspection_to_one_hz() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};
        let mut app = sample_state();
        let central = app.build_central_panel_snapshot();
        let group = &central.groups[0];
        let program = &group.programs[0];
        assert!(app.runtime.add_running_app_with_token(
            &program.app_key,
            701,
            1001,
            group.group_id.clone(),
            program.rule_id.clone(),
        ));
        let inspections = Cell::new(0);
        let inspect = |pid| {
            inspections.set(inspections.get() + 1);
            assert_eq!(pid, 701);
            Ok((PathBuf::from("sample.exe"), 1001))
        };
        let start = Instant::now();
        app.refresh_tracked_processes_with_inspector(start, inspect);
        let first = app.ui.tracked_processes.rows.clone();
        assert_eq!(first.len(), 1);
        let pointer = app.ui.tracked_processes.rows.as_ptr();
        for millis in 1..1000 {
            app.refresh_tracked_processes_with_inspector(
                start + Duration::from_millis(millis),
                inspect,
            );
        }
        assert_eq!(
            inspections.get(),
            1,
            "GUI frames must not repeat native inspections"
        );
        assert_eq!(app.ui.tracked_processes.rows.as_ptr(), pointer);

        let handle = app.runtime.running_apps_handle();
        let writer = handle.try_write().unwrap();
        app.refresh_tracked_processes_with_inspector(start + Duration::from_secs(1), inspect);
        assert_eq!(
            app.ui.tracked_processes.rows, first,
            "Busy is not an empty roster"
        );
        assert!(app.ui.tracked_processes.updating);
        drop(writer);
        app.refresh_tracked_processes_with_inspector(start + Duration::from_millis(1500), inspect);
        assert_eq!(
            inspections.get(),
            1,
            "Busy retries also respect the one-second cadence"
        );
        app.refresh_tracked_processes_with_inspector(start + Duration::from_secs(2), inspect);
        assert_eq!(inspections.get(), 2);
        assert!(!app.ui.tracked_processes.updating);
        assert_eq!(
            app.ui.tracked_processes.rows.as_ptr(),
            pointer,
            "unchanged rows keep their allocation"
        );

        handle
            .try_write()
            .unwrap()
            .mutate_membership(|apps| apps.remove_app(&program.app_key));
        app.refresh_tracked_processes_with_inspector(start + Duration::from_secs(3), inspect);
        assert!(
            app.ui.tracked_processes.rows.is_empty(),
            "a completed empty snapshot must clear old rows"
        );
        assert!(!app.ui.tracked_processes.updating);
        assert_eq!(inspections.get(), 2);
    }

    #[test]
    fn tracked_process_snapshot_lists_only_runtime_confirmed_pids() {
        let mut app = sample_state();
        let central = app.build_central_panel_snapshot();
        let group = &central.groups[0];
        let program = &group.programs[0];
        assert!(app.runtime.add_running_app_with_token(
            &program.app_key,
            701,
            1001,
            group.group_id.clone(),
            program.rule_id.clone(),
        ));
        for (pid, token) in [(702, 1002), (703, 1003), (704, 1004)] {
            assert!(app
                .runtime
                .add_pid_to_existing_app_with_token(&program.app_key, pid, token));
        }

        assert_eq!(
            app.tracked_processes_snapshot_with_inspector(|pid| match pid {
                701 => Ok((PathBuf::from("apps/sample.exe"), 1001)),
                // A live PID reused by an unrelated process is not a tracked instance.
                702 => Ok((PathBuf::from("apps/unrelated.exe"), 9002)),
                703 => Err("exited or inaccessible".to_string()),
                704 => Ok((PathBuf::from("apps/helper.exe"), 1004)),
                _ => panic!("must not inspect untracked processes"),
            })
            .unwrap(),
            vec![
                TrackedProcessSnapshot {
                    group_name: "Games".to_string(),
                    process_name: "sample.exe".to_string(),
                    pid: 701,
                },
                TrackedProcessSnapshot {
                    group_name: "Games".to_string(),
                    process_name: "helper.exe".to_string(),
                    pid: 704,
                },
            ]
        );
    }

    #[test]
    fn tracked_process_snapshot_uses_only_the_runtime_rule_owner() {
        let mut app = sample_state();
        {
            let mut storage = app.persistent_state.write().unwrap();
            let mut second = storage.groups[0].clone();
            second.name = "Background".into();
            let duplicate = second.programs[0].clone();
            second.programs.push(duplicate);
            storage.groups.push(second);
        }
        let central = app.build_central_panel_snapshot();
        let owner_group = &central.groups[1];
        // Own the second duplicate rule, not the first matching configured key.
        let owner = &owner_group.programs[1];
        assert!(app.runtime.add_running_app_with_token(
            &owner.app_key,
            701,
            1001,
            owner_group.group_id.clone(),
            owner.rule_id.clone(),
        ));
        let mut inspections = 0;
        let rows = app
            .tracked_processes_snapshot_with_inspector(|pid| {
                inspections += 1;
                assert_eq!(pid, 701);
                Ok((PathBuf::from("sample.exe"), 1001))
            })
            .unwrap();
        assert_eq!(
            rows,
            vec![TrackedProcessSnapshot {
                group_name: "Background".into(),
                process_name: "sample.exe".into(),
                pid: 701,
            }]
        );
        assert_eq!(inspections, 1);

        // A removed owner must not be relabeled as another rule with the same key.
        app.persistent_state.write().unwrap().groups[1]
            .programs
            .pop();
        assert!(app
            .tracked_processes_snapshot_with_inspector(|_| {
                panic!("stale runtime ownership must not be inspected")
            })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn tracked_process_snapshot_skips_unverified_and_busy_runtime_entries() {
        let mut app = sample_state();
        let central = app.build_central_panel_snapshot();
        let group = &central.groups[0];
        let program = &group.programs[0];
        assert!(app.runtime.add_running_app(
            &program.app_key,
            701,
            group.group_id.clone(),
            program.rule_id.clone(),
        ));
        assert!(app
            .tracked_processes_snapshot_with_inspector(|_| {
                panic!("a PID without an instance token must not be inspected")
            })
            .unwrap()
            .is_empty());

        let handle = app.runtime.running_apps_handle();
        let _writer = handle.try_write().unwrap();
        assert!(app
            .tracked_processes_snapshot_with_inspector(|_| {
                panic!("a busy runtime must not fall back to unverified PIDs")
            })
            .is_none());
    }

    #[test]
    fn test_installed_app_picker_open_query_navigation_and_close() {
        let mut app = sample_state();
        app.ui.installed_app_picker.entries = vec![
            InstalledAppCatalogEntry::new_aumid(
                "Spotify",
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
                InstalledAppCatalogSource::WindowsAppsFolder,
            ),
            InstalledAppCatalogEntry::new_aumid(
                "Steam",
                "ValveCorporation.Steam!Steam",
                InstalledAppCatalogSource::WindowsAppsFolder,
            ),
        ];

        app.open_installed_app_picker(group_id(&app, 0));
        assert_eq!(
            app.ui.installed_app_picker.target_group_id,
            Some(group_id(&app, 0))
        );
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::InstalledAppPicker
        ));
        assert!(app.take_installed_app_picker_focus_request());
        assert!(!app.take_installed_app_picker_focus_request());

        app.set_installed_app_picker_query("steam".into());
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(1));

        app.select_previous_installed_app_picker_entry();
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(1));

        app.set_installed_app_picker_query(String::new());
        app.select_next_installed_app_picker_entry();
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(1));
        app.select_previous_installed_app_picker_entry();
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(0));

        app.close_installed_app_picker();
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.installed_app_picker.target_group_id.is_none());
        assert!(app.ui.installed_app_picker.query.is_empty());
    }

    #[test]
    fn test_confirm_selected_installed_app_adds_entry_and_saves_once() {
        let mut app = sample_state();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry::new_aumid(
            "Spotify",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            InstalledAppCatalogSource::WindowsAppsFolder,
        )];

        app.open_installed_app_picker(group_id(&app, 0));
        assert!(app.confirm_selected_installed_app());
        app.wait_for_test_save();

        let state = app.persistent_state.read().unwrap();
        let added = &state.groups[0].programs[1];
        assert!(matches!(
            added.launch_target,
            LaunchTarget::Installed { .. }
        ));
        assert_eq!(
            added.installed_aumid(),
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify")
        );
        drop(state);
        assert_eq!(app.save_count(), 1);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.installed_app_picker.target_group_id.is_none());
    }

    struct ImportPersistence {
        fail: Arc<std::sync::atomic::AtomicBool>,
        gate: Option<std::sync::mpsc::Receiver<()>>,
        entered: Option<std::sync::mpsc::Sender<()>>,
    }

    impl crate::app::features::system_tweaks::StatePersistence for ImportPersistence {
        fn save(
            &mut self,
            _: &mut AppStateStorage,
        ) -> Result<crate::app::features::system_tweaks::service::SavedState, super::SaveError>
        {
            if let Some(gate) = self.gate.take() {
                self.entered.take().unwrap().send(()).unwrap();
                gate.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            }
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                Err(super::SaveError::Storage)
            } else {
                Ok(crate::app::features::system_tweaks::service::SavedState::Unavailable)
            }
        }
    }

    struct ImportGate(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for ImportGate {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    fn install_import_persistence(
        app: &mut AppState,
        pending: bool,
    ) -> (Arc<std::sync::atomic::AtomicBool>, ImportGate) {
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let state = app.persistent_state.read().unwrap().clone();
        let runtime = crate::app::features::system_tweaks::TuningRuntime::start(
            crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                .headless(
                    state,
                    Box::new(ImportPersistence {
                        fail: fail.clone(),
                        gate: pending.then_some(release_rx),
                        entered: pending.then_some(entered_tx),
                    }),
                ),
            app.runtime.running_apps_handle(),
        );
        app.replace_tuning_for_test(runtime, Default::default());
        if pending {
            let mut candidate = app.persistent_state.read().unwrap().clone();
            candidate.mark_ready_for_current_schema_save(app.rules.to_persisted_identities());
            app.request_tuning(
                crate::app::features::system_tweaks::TuningRequest::SaveBase {
                    candidate: Box::new(candidate),
                    expected_revision: app.tuning_adopted_revision,
                },
            )
            .unwrap();
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }
        (fail, ImportGate(pending.then_some(release_tx)))
    }

    fn await_import_completion(app: &mut AppState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while matches!(
            app.tuning_snapshot().operation,
            crate::app::features::system_tweaks::TuningOperationState::Pending { .. }
        ) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.poll_tuning();
    }

    fn assert_import_not_saved(
        app: &AppState,
        identities: &crate::app::features::rules::PersistedRuleIdentities,
    ) {
        assert_eq!(
            app.persistent_state.read().unwrap().groups[0]
                .programs
                .len(),
            1
        );
        assert_eq!(&app.rules.to_persisted_identities(), identities);
        assert!(!app
            .log_manager
            .entries
            .iter()
            .any(|entry| entry.message.starts_with("Added ")));
    }

    fn assert_one_import_saved(app: &AppState, old_group: &GroupId, old_rule: &RuleId) {
        assert_eq!(
            app.persistent_state.read().unwrap().groups[0]
                .programs
                .len(),
            2
        );
        assert_eq!(app.rules.group_id_for_index(0).as_ref(), Some(old_group));
        assert_eq!(app.rules.rule_id_for_index(0, 0).as_ref(), Some(old_rule));
        assert_ne!(app.rules.rule_id_for_index(0, 1).as_ref(), Some(old_rule));
        assert_eq!(
            app.log_manager
                .entries
                .iter()
                .filter(|entry| entry.message.starts_with("Added "))
                .count(),
            1
        );
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn import_retention_dropped_pending_retry() {
        let mut app = sample_state();
        let (_fail, gate) = install_import_persistence(&mut app, true);
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        let files = vec![PathBuf::from(r"C:\Dropped.exe")];
        app.ui.dropped_files = Some(files.clone());
        app.ui.file_drop_hover_target = Some(group.clone());
        assert!(!app.consume_dropped_files_into_group(group.clone()));
        assert_eq!(app.ui.dropped_files.as_ref(), Some(&files));
        assert_eq!(app.ui.file_drop_hover_target.as_ref(), Some(&group));
        assert_import_not_saved(&app, &identities);
        drop(gate);
        await_import_completion(&mut app);
        assert!(app.consume_dropped_files_into_group(group.clone()));
        await_import_completion(&mut app);
        assert!(app.ui.dropped_files.is_none());
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[test]
    fn import_retention_installed_pending_retry() {
        let mut app = sample_state();
        let (_fail, gate) = install_import_persistence(&mut app, true);
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry::new_aumid(
            "Spotify",
            "Spotify!App",
            InstalledAppCatalogSource::WindowsAppsFolder,
        )];
        app.open_installed_app_picker(group.clone());
        app.set_installed_app_picker_query("spot".into());
        assert!(!app.confirm_selected_installed_app());
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::InstalledAppPicker
        ));
        assert_eq!(app.ui.installed_app_picker.query, "spot");
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(0));
        assert_import_not_saved(&app, &identities);
        drop(gate);
        await_import_completion(&mut app);
        assert!(app.confirm_selected_installed_app());
        await_import_completion(&mut app);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn import_retention_dropped_storage_failure_retry() {
        let mut app = sample_state();
        let (fail, _gate) = install_import_persistence(&mut app, false);
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        let files = vec![PathBuf::from(r"C:\Dropped.exe")];
        app.ui.dropped_files = Some(files.clone());
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = app.consume_dropped_files_into_group(group.clone());
        await_import_completion(&mut app);
        assert_eq!(app.ui.dropped_files.as_ref(), Some(&files));
        assert_import_not_saved(&app, &identities);
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(app.consume_dropped_files_into_group(group.clone()));
        await_import_completion(&mut app);
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[test]
    fn import_retention_installed_storage_failure_retry() {
        let mut app = sample_state();
        let (fail, _gate) = install_import_persistence(&mut app, false);
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry::new_aumid(
            "Spotify",
            "Spotify!App",
            InstalledAppCatalogSource::WindowsAppsFolder,
        )];
        app.open_installed_app_picker(group.clone());
        app.set_installed_app_picker_query("spot".into());
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = app.confirm_selected_installed_app();
        await_import_completion(&mut app);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::InstalledAppPicker
        ));
        assert_eq!(
            app.ui.installed_app_picker.target_group_id.as_ref(),
            Some(&group)
        );
        assert_eq!(app.ui.installed_app_picker.query, "spot");
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(0));
        assert_import_not_saved(&app, &identities);
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(app.confirm_selected_installed_app());
        await_import_completion(&mut app);
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[test]
    fn import_retention_accepted_installed_waits_for_adoption() {
        let mut app = sample_state();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let state = app.persistent_state.read().unwrap().clone();
        let runtime = crate::app::features::system_tweaks::TuningRuntime::start(
            crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                .headless(
                    state,
                    Box::new(ImportPersistence {
                        fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        gate: Some(release_rx),
                        entered: Some(entered_tx),
                    }),
                ),
            app.runtime.running_apps_handle(),
        );
        app.replace_tuning_for_test(runtime, Default::default());
        let gate = ImportGate(Some(release_tx));
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry::new_aumid(
            "Spotify",
            "Spotify!App",
            InstalledAppCatalogSource::WindowsAppsFolder,
        )];
        app.open_installed_app_picker(group.clone());
        app.set_installed_app_picker_query("spot".into());
        // Queue acceptance returns before the gated save can complete.
        let _ = app.confirm_selected_installed_app();
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            app.tuning_snapshot().operation,
            crate::app::features::system_tweaks::TuningOperationState::Pending { .. }
        ));
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::InstalledAppPicker
        ));
        assert_eq!(app.ui.installed_app_picker.query, "spot");
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(0));
        assert_import_not_saved(&app, &identities);
        assert!(!app.confirm_selected_installed_app());
        drop(gate);
        await_import_completion(&mut app);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn import_retention_accepted_dropped_preserves_new_target() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Other group");
        let group = group_id(&app, 0);
        let other_group = group_id(&app, 1);
        let old_rule = rule_id(&app, 0, 0);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let state = app.persistent_state.read().unwrap().clone();
        let runtime = crate::app::features::system_tweaks::TuningRuntime::start(
            crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                .headless(
                    state,
                    Box::new(ImportPersistence {
                        fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        gate: Some(release_rx),
                        entered: Some(entered_tx),
                    }),
                ),
            app.runtime.running_apps_handle(),
        );
        app.replace_tuning_for_test(runtime, Default::default());
        let gate = ImportGate(Some(release_tx));
        let files = vec![PathBuf::from(r"C:\Dropped.exe")];
        app.ui.dropped_files = Some(files.clone());
        let _ = app.consume_dropped_files_into_group(group.clone());
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            app.tuning_snapshot().operation,
            crate::app::features::system_tweaks::TuningOperationState::Pending { .. }
        ));
        // A fresh drop of the same files now belongs to another group.
        app.ui.dropped_files = Some(files.clone());
        app.ui.file_drop_hover_target = Some(other_group.clone());
        drop(gate);
        await_import_completion(&mut app);
        assert_one_import_saved(&app, &group, &old_rule);
        assert_eq!(app.ui.dropped_files.as_ref(), Some(&files));
        assert_eq!(app.ui.file_drop_hover_target.as_ref(), Some(&other_group));
        assert!(app.persistent_state.read().unwrap().groups[1]
            .programs
            .is_empty());
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn import_retention_accepted_dropped_clears_transient_hover() {
        let mut app = sample_state();
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let state = app.persistent_state.read().unwrap().clone();
        let runtime = crate::app::features::system_tweaks::TuningRuntime::start(
            crate::app::features::system_tweaks::Tuning::from_registry(Default::default())
                .headless(
                    state,
                    Box::new(ImportPersistence {
                        fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        gate: Some(release_rx),
                        entered: Some(entered_tx),
                    }),
                ),
            app.runtime.running_apps_handle(),
        );
        app.replace_tuning_for_test(runtime, Default::default());
        let gate = ImportGate(Some(release_tx));
        app.ui.dropped_files = Some(vec![PathBuf::from(r"C:\Dropped.exe")]);
        let _ = app.consume_dropped_files_into_group(group.clone());
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            app.tuning_snapshot().operation,
            crate::app::features::system_tweaks::TuningOperationState::Pending { .. }
        ));
        // The pointer moved outside group rows; this is not a fresh drop.
        app.ui.file_drop_hover_target = None;
        drop(gate);
        await_import_completion(&mut app);
        assert_one_import_saved(&app, &group, &old_rule);
        assert!(
            app.ui.dropped_files.is_none(),
            "accepted input must not survive as a second drop"
        );
        assert!(app.ui.file_drop_hover_target.is_none());
        assert!(!app.consume_dropped_files_into_group(group.clone()));
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn import_retention_selected_files_pending_retry() {
        let mut app = sample_state();
        let (_fail, gate) = install_import_persistence(&mut app, true);
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        let files = vec![PathBuf::from(r"C:\Selected.exe")];
        app.add_selected_files_to_group(group.clone(), files.clone());
        assert_eq!(app.ui.dropped_files.as_ref(), Some(&files));
        assert_eq!(app.ui.file_drop_hover_target.as_ref(), Some(&group));
        assert_import_not_saved(&app, &identities);
        drop(gate);
        await_import_completion(&mut app);
        assert!(app.consume_dropped_files_into_group(group.clone()));
        await_import_completion(&mut app);
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn import_retention_selected_files_storage_failure_retry() {
        let mut app = sample_state();
        let (fail, _gate) = install_import_persistence(&mut app, false);
        let group = group_id(&app, 0);
        let old_rule = rule_id(&app, 0, 0);
        let identities = app.rules.to_persisted_identities();
        let files = vec![PathBuf::from(r"C:\Selected.exe")];
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        app.add_selected_files_to_group(group.clone(), files.clone());
        await_import_completion(&mut app);
        assert_eq!(app.ui.dropped_files.as_ref(), Some(&files));
        assert_eq!(app.ui.file_drop_hover_target.as_ref(), Some(&group));
        assert_import_not_saved(&app, &identities);
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(app.consume_dropped_files_into_group(group.clone()));
        await_import_completion(&mut app);
        assert_one_import_saved(&app, &group, &old_rule);
    }

    #[test]
    fn test_leaving_picker_route_clears_session_but_keeps_cached_entries() {
        let mut app = sample_state();
        let (_tx, rx) = std::sync::mpsc::channel();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry::new_aumid(
            "Spotify",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            InstalledAppCatalogSource::WindowsAppsFolder,
        )];
        app.open_installed_app_picker(group_id(&app, 0));
        app.ui.installed_app_picker.query = "spot".into();
        app.ui.installed_app_picker.last_error = Some("boom".into());
        app.ui.installed_app_picker.is_refreshing = true;
        app.ui.installed_app_picker.refresh_rx = Some(rx);

        app.set_current_window(WindowRoute::Logs);

        assert!(matches!(app.ui.current_window, WindowRoute::Logs));
        assert!(app.ui.installed_app_picker.target_group_id.is_none());
        assert!(app.ui.installed_app_picker.query.is_empty());
        assert!(app.ui.installed_app_picker.last_error.is_none());
        assert!(!app.ui.installed_app_picker.is_refreshing);
        assert!(app.ui.installed_app_picker.refresh_rx.is_none());
        assert_eq!(app.ui.installed_app_picker.entries.len(), 1);
    }

    #[test]
    fn test_leaving_app_run_settings_route_clears_edit_session() {
        let mut app = sample_state();
        let target = RuleEditorTarget {
            group_id: group_id(&app, 0),
            rule_id: rule_id(&app, 0, 0),
        };
        app.ui.current_window = WindowRoute::AppRunSettings;
        app.ui.app_edit_state.target = Some(target);
        app.ui.app_edit_state.current_edit = Some(sample_app("Draft"));
        app.ui.app_edit_state.shortcut_result = Some(RuleShortcutResult::Failed {
            message: "old status".to_string(),
        });

        app.set_current_window(WindowRoute::Logs);

        assert!(matches!(app.ui.current_window, WindowRoute::Logs));
        assert!(app.ui.app_edit_state.target.is_none());
        assert!(app.ui.app_edit_state.current_edit.is_none());
        assert!(app.ui.app_edit_state.shortcut_result.is_none());
    }

    #[test]
    fn test_linux_path_entries_stay_hidden_until_query() {
        let mut app = sample_state();
        app.ui.installed_app_picker.entries = vec![
            InstalledAppCatalogEntry::new_path(
                "Steam",
                PathBuf::from("/usr/share/applications/steam.desktop"),
                InstalledAppCatalogSource::LinuxDesktopEntry,
            )
            .with_detail("/usr/bin/steam"),
            InstalledAppCatalogEntry::new_path(
                "steamcmd",
                PathBuf::from("/usr/bin/steamcmd"),
                InstalledAppCatalogSource::LinuxPathExecutable,
            ),
        ];

        app.open_installed_app_picker(group_id(&app, 0));
        let snapshot = app.build_installed_app_picker_snapshot();
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(snapshot.rows[0].name, "Steam");

        app.set_installed_app_picker_query("steam".into());
        let snapshot = app.build_installed_app_picker_snapshot();
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(snapshot.rows[0].name, "Steam");
        assert_eq!(snapshot.rows[1].name, "steamcmd");
    }

    #[test]
    fn test_installed_app_picker_ranks_exact_before_prefix_and_detail_matches() {
        let mut app = sample_state();
        app.ui.installed_app_picker.entries = vec![
            InstalledAppCatalogEntry::new_path(
                "code",
                PathBuf::from("/usr/bin/code"),
                InstalledAppCatalogSource::LinuxPathExecutable,
            ),
            InstalledAppCatalogEntry::new_path(
                "code-server",
                PathBuf::from("/usr/bin/code-server"),
                InstalledAppCatalogSource::LinuxPathExecutable,
            ),
            InstalledAppCatalogEntry::new_path(
                "Visual Studio",
                PathBuf::from("/usr/share/applications/code.desktop"),
                InstalledAppCatalogSource::LinuxDesktopEntry,
            )
            .with_detail("/usr/bin/code"),
        ];

        app.open_installed_app_picker(group_id(&app, 0));
        app.set_installed_app_picker_query("code".into());

        let snapshot = app.build_installed_app_picker_snapshot();
        let names: Vec<String> = snapshot.rows.into_iter().map(|row| row.name).collect();
        assert_eq!(names, vec!["code", "code-server", "Visual Studio"]);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn event_log_diagnostics_default_on_without_starting_before_shell_gate() {
        let mut app = sample_state();
        let state = app.persistent_state.clone();

        assert!(app.windows_event_log_diagnostics_enabled());
        assert!(!app.windows_event_log_worker_is_active());

        app.set_windows_event_log_diagnostics_enabled(false)
            .unwrap();
        app.wait_for_test_save();

        assert!(!state.read().unwrap().windows_event_log_diagnostics_enabled);
        assert!(!app.windows_event_log_worker_is_active());
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn session_only_disable_blocks_an_older_persisted_opt_in() {
        let mut app = sample_state();
        {
            let mut state = app.persistent_state.write().unwrap();
            state.windows_event_log_diagnostics_enabled = true;
        }
        app.windows_event_log_disabled_for_session = true;
        app.windows_event_log.enable();

        assert!(!app.start_windows_event_log_scan_after_shell_gate());
        assert!(!app.windows_event_log_worker_is_active());
    }
}
