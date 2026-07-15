use crate::app::adapters::storage::StorageAdapter;
use crate::app::features::execution::{self, RuntimeRegistry};
use crate::app::features::preferences;
use crate::app::features::rules::{self, RulesContext};
use crate::app::features::shortcut::{
    create_saved_rule_shortcut, CreateRuleShortcutError, RuleShortcutPlatform,
    SystemRuleShortcutPlatform,
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

/// Facade combining persisted, transient UI, and runtime tracking state.
pub struct AppState {
    pub(crate) persistent_state: Arc<RwLock<AppStateStorage>>,
    pub(crate) rules: RulesContext,
    pub(crate) ui: UiSession,
    pub(crate) runtime: RuntimeRegistry,
    pub(crate) log_manager: LogManager,
    shortcut_creation_role: ShortcutCreationRole,
    #[cfg(test)]
    save_count: usize,
}

impl AppState {
    pub fn new() -> Self {
        let storage = StorageAdapter::load();
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
        Self {
            persistent_state,
            rules,
            ui: UiSession::new(effective_total_threads()),
            runtime: RuntimeRegistry::new(),
            log_manager: LogManager::default(),
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
        Self {
            persistent_state,
            rules,
            ui: UiSession::new(total_threads),
            runtime: RuntimeRegistry::new(),
            log_manager: LogManager::default(),
            shortcut_creation_role: default_shortcut_creation_role(),
            save_count: 0,
        }
    }

    #[cfg_attr(not(feature = "windows"), allow(dead_code))]
    pub(crate) fn set_shortcut_creation_role(&mut self, role: ShortcutCreationRole) {
        self.shortcut_creation_role = role;
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

    fn persist_state(&mut self) -> bool {
        self.reconcile_rules();

        if let Ok(mut state) = self.persistent_state.write() {
            state.mark_ready_for_current_schema_save(self.rules.to_persisted_identities());
        } else {
            self.log_manager
                .add_sticky_once("WARNING: persistent_state lock poisoned during save".into());
            return false;
        }

        #[cfg(test)]
        {
            self.save_count += 1;
            true
        }

        #[cfg(not(test))]
        {
            let save_result = match self.persistent_state.write() {
                Ok(mut state) => state.try_save_state(),
                Err(_) => {
                    self.log_manager.add_sticky_once(
                        "WARNING: persistent_state lock poisoned during save".into(),
                    );
                    return false;
                }
            };

            if let Err(err) = save_result {
                self.log_manager
                    .add_important_sticky_once(format!("ERROR: Failed to save state: {err}"));
                return false;
            }

            true
        }
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
                groups: self
                    .rules
                    .snapshot(&state)
                    .groups
                    .into_iter()
                    .map(|group| CentralGroupSnapshot {
                        group_id: group.id,
                        name: group.name,
                        cores: group.cores,
                        is_hidden: group.is_hidden,
                        run_all_button: group.run_all_enabled,
                        programs: group
                            .rules
                            .iter()
                            .map(|program| CentralProgramSnapshot {
                                rule_id: program.id.clone(),
                                name: program.app.name.clone(),
                                launch_target_detail: program.app.launch_target_detail(),
                                app_key: program.app.get_key(),
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

        if rules::set_group_is_hidden(&self.persistent_state, group_index, is_hidden) {
            let _ = self.persist_state();
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

        let moved = rules::move_group_to_index(&self.persistent_state, source_index, target_index);
        if moved {
            self.rules.move_group_to_index(source_index, target_index);
            let _ = self.persist_state();
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

        let (moving_key, moving_name, target_group_name) = match read_result {
            Ok(values) => values,
            Err(message) => {
                self.log_manager.add_entry(message);
                return MoveRuleToGroupOutcome::DuplicateInTarget;
            }
        };

        let moved = match self.persistent_state.write() {
            Ok(mut state) => {
                let duplicate_exists = source_group_index != target_group_index
                    && state
                        .groups
                        .get(target_group_index)
                        .map(|group| {
                            group
                                .programs
                                .iter()
                                .any(|program| program.get_key() == moving_key)
                        })
                        .unwrap_or(false);
                if duplicate_exists {
                    return MoveRuleToGroupOutcome::DuplicateInTarget;
                }

                rules::move_rule_between_groups_at(
                    &mut state,
                    source_group_index,
                    source_rule_index,
                    target_group_index,
                    target_rule_index,
                )
            }
            Err(_) => return MoveRuleToGroupOutcome::LockFailed,
        };

        if moved.is_none() {
            return MoveRuleToGroupOutcome::MissingRule;
        }
        if self
            .rules
            .move_rule_between_groups_at(
                source_group_index,
                source_rule_index,
                target_group_index,
                target_rule_index,
            )
            .is_none()
        {
            self.reconcile_rules();
            self.log_manager.add_important_sticky_once(
                "ERROR: Failed to move rule identity after moving app rule".to_string(),
            );
            return MoveRuleToGroupOutcome::LockFailed;
        }

        let _ = self.persist_state();
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

        let attempted_count = paths.len();
        let group_name = self.get_group_name(group_index).unwrap_or_default();
        self.log_manager.add_entry(format!(
            "Adding app targets to group: {group_name}, paths: {paths:?}"
        ));

        let outcome = rules::add_apps_to_group(&self.persistent_state, group_index, paths);
        self.handle_add_apps_outcome(group_index, &group_name, attempted_count, outcome);
    }

    pub fn add_installed_app_to_group(
        &mut self,
        group_id: GroupId,
        entry: InstalledAppCatalogEntry,
    ) {
        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return;
        };

        let group_name = self.get_group_name(group_index).unwrap_or_default();
        let app_name = entry.name.clone();
        let outcome = rules::add_installed_app_to_group(&self.persistent_state, group_index, entry);

        if outcome.added_count > 0 {
            self.rules
                .append_rules_to_group(group_index, outcome.added_count);
            let _ = self.persist_state();
            self.log_manager.add_entry(format!(
                "Added installed app '{app_name}' to group: {group_name}"
            ));
        }

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding installed app '{app_name}': {err}"));
        }
    }

    pub fn consume_dropped_files_into_group(&mut self, group_id: GroupId) -> bool {
        self.ui.file_drop_hover_target = None;
        let Some(files) = self.ui.dropped_files.take() else {
            return false;
        };

        if files.is_empty() {
            return false;
        }

        let Some(group_index) = self.group_index_for_id(&group_id) else {
            return false;
        };

        let files_count = files.len();
        let group_name = self.get_group_name(group_index).unwrap_or_default();

        let outcome = rules::add_apps_to_group(&self.persistent_state, group_index, files);
        self.handle_add_apps_outcome(group_index, &group_name, files_count, outcome);

        true
    }

    fn handle_add_apps_outcome(
        &mut self,
        group_index: usize,
        group_name: &str,
        attempted_count: usize,
        outcome: AddAppsOutcome,
    ) {
        if outcome.added_count > 0 {
            self.rules
                .append_rules_to_group(group_index, outcome.added_count);
            let _ = self.persist_state();

            if outcome.added_count == attempted_count {
                self.log_manager
                    .add_entry(format!("Added app targets to group: {group_name}"));
            } else {
                self.log_manager.add_entry(format!(
                    "Added {} app targets to group: {}",
                    outcome.added_count, group_name
                ));
            }
        }

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding app targets: {err}"));
        }
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

    pub fn toggle_theme(&mut self) {
        preferences::toggle_theme(&self.persistent_state);
        let _ = self.persist_state();
    }

    pub fn toggle_process_monitoring(&mut self) {
        preferences::toggle_process_monitoring(&self.persistent_state);
        let _ = self.persist_state();
    }

    pub fn is_process_monitoring_enabled(&self) -> bool {
        self.persistent_state
            .read()
            .unwrap()
            .process_monitoring_enabled
    }

    pub fn commit_group_form_session(&mut self) {
        let should_save = if let Some(group_id) = self.ui.group_form.editing_group_id.clone() {
            let Some(index) = self.group_index_for_id(&group_id) else {
                self.ui.reset_group_form();
                self.ui
                    .set_current_window(WindowRoute::Groups(GroupRoute::List));
                return;
            };

            match rules::update_group_properties(
                &self.persistent_state,
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
                &self.persistent_state,
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
            if self.ui.group_form.editing_group_id.is_none() {
                self.rules.append_group();
            }
            let _ = self.persist_state();
        }

        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn delete_current_group_form_target(&mut self) {
        if let Some(group_id) = self.ui.group_form.editing_group_id.clone() {
            if let Some(index) = self.group_index_for_id(&group_id) {
                if rules::remove_group(&self.persistent_state, index) {
                    self.rules.remove_group(index);
                    let _ = self.persist_state();
                }
            }
        }

        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn cancel_group_form_session(&mut self) {
        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        let leaving_installed_app_picker =
            matches!(self.ui.current_window, WindowRoute::InstalledAppPicker)
                && !matches!(window, WindowRoute::InstalledAppPicker);
        let leaving_app_run_settings =
            matches!(self.ui.current_window, WindowRoute::AppRunSettings)
                && !matches!(window, WindowRoute::AppRunSettings);

        if leaving_installed_app_picker {
            self.reset_installed_app_picker_session();
        }
        if leaving_app_run_settings {
            self.reset_app_run_settings_session();
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

        std::thread::spawn(move || {
            let _ = tx.send(crate::app::adapters::discovery::list_supported_start_apps());
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

        self.add_installed_app_to_group(group_id, entry);
        self.close_installed_app_picker();
        true
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

                if rules::update_rule(&self.persistent_state, group_idx, prog_idx, updated_app) {
                    let _ = self.persist_state();
                }
            }
        }

        self.close_app_run_settings();
    }

    pub fn delete_current_app_edit_target(&mut self) {
        if let Some(target) = self.ui.app_edit_state.target.clone() {
            if let Some((group_idx, prog_idx)) =
                self.rule_indices_for_ids(&target.group_id, &target.rule_id)
            {
                if let Some(path) =
                    rules::remove_rule_from_group(&self.persistent_state, group_idx, prog_idx)
                {
                    self.rules.remove_rule(group_idx, prog_idx);
                    let _ = self.persist_state();
                    self.log_manager
                        .add_entry(format!("Removing app: {}", path));
                }
            }
        }

        self.close_app_run_settings();
    }

    pub fn clear_logs(&mut self) {
        self.log_manager.clear();
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
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use super::RuleShortcutDisabledReason;
    use super::{AppState, MoveRuleToGroupOutcome, RunRuleOutcome};
    use crate::app::features::execution::RuntimeRegistry;
    use crate::app::features::rules::RulesContext;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    use crate::app::features::shortcut::{
        CreateRuleShortcutError, RuleShortcutPlatform, ShortcutWriteError,
    };
    use crate::app::models::{
        AppStateStorage, AppToRun, CoreGroup, CpuSchema, LaunchTarget, LogManager,
    };
    use crate::app::shared::ids::{GroupId, RuleId};
    use crate::app::shell::sessions::{RuleEditorTarget, RuleShortcutResult, ShortcutCreationRole};
    use crate::app::shell::UiSession;
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
        let persistent_state = Arc::new(RwLock::new(AppStateStorage {
            version: 5,
            groups: vec![CoreGroup {
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
            }],
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }));

        let rules = persistent_state
            .read()
            .map(|state| RulesContext::from_storage(&state))
            .unwrap();

        AppState {
            persistent_state,
            rules,
            ui: UiSession::new(4),
            runtime: RuntimeRegistry::new(),
            log_manager: LogManager::default(),
            shortcut_creation_role: ShortcutCreationRole::Primary,
            save_count: 0,
        }
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

        assert_eq!(
            app.run_group_program(source_group_id, moved_rule_id),
            RunRuleOutcome::MissingRule
        );
    }

    #[test]
    fn test_run_group_program_reports_accepted_for_existing_rule() {
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

        assert_eq!(
            app.run_group_program(existing_group_id, existing_rule_id),
            RunRuleOutcome::Accepted
        );
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
    fn test_commit_group_form_session_preserves_invalid_create_closeout() {
        let mut app = sample_state();
        app.ui.current_window = WindowRoute::Groups(GroupRoute::Create);

        app.commit_group_form_session();

        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
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

        let source_group_id = group_id(&app, 0);
        let target_group_id = group_id(&app, 1);
        let moved_rule_id = rule_id(&app, 0, 0);
        let original_app = app.persistent_state.read().unwrap().groups[0].programs[0].clone();

        let outcome =
            app.move_rule_to_group_at(source_group_id, moved_rule_id.clone(), target_group_id, 1);

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
        let group_id = group_id(&app, 0);
        let first_rule_id = rule_id(&app, 0, 0);
        let second_rule_id = rule_id(&app, 0, 1);
        let third_rule_id = rule_id(&app, 0, 2);

        let outcome =
            app.move_rule_to_group_at(group_id.clone(), first_rule_id.clone(), group_id, 3);

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
        let state = app.persistent_state.read().unwrap();
        assert_eq!(state.groups[0].programs.len(), 1);
        assert_eq!(state.groups[1].programs.len(), 1);
        drop(state);
        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_toggle_theme_and_monitoring_save_once() {
        let mut app = sample_state();

        app.toggle_theme();
        assert_eq!(app.get_theme_index(), 1);
        assert_eq!(app.save_count(), 1);

        app.toggle_process_monitoring();
        assert!(app.is_process_monitoring_enabled());
        assert_eq!(app.save_count(), 2);
    }

    #[test]
    fn test_set_group_hidden_saves_only_on_real_change() {
        let mut app = sample_state();
        let group_id = group_id(&app, 0);

        app.set_group_is_hidden(group_id.clone(), true);
        assert!(app.persistent_state.read().unwrap().groups[0].is_hidden);
        assert_eq!(app.save_count(), 1);

        app.set_group_is_hidden(group_id, true);
        assert_eq!(app.save_count(), 1);

        app.set_group_is_hidden(GroupId("missing-group".to_string()), false);
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_move_group_invalid_target_or_missing_group_do_not_save() {
        let mut app = sample_state();
        add_empty_group(&mut app, "Work");
        let first_group_id = group_id(&app, 0);

        assert!(!app.move_group_to_index(first_group_id, 0));
        assert!(!app.move_group_to_index(GroupId("missing-group".to_string()), 1));
        assert!(!app.move_group_to_index(group_id(&app, 0), 99));

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
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 2);
        assert_eq!(app.save_count(), 1);

        app.ui.group_form.editing_group_id = Some(group_id(&app, 1));
        app.delete_current_group_form_target();
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

        assert!(app.persistent_state.read().unwrap().groups[0]
            .programs
            .is_empty());
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_noop_delete_current_app_edit_target_does_not_save() {
        let mut app = sample_state();

        app.delete_current_app_edit_target();

        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_consume_dropped_files_without_pending_files_clears_cached_target() {
        let mut app = sample_state();
        let target_group_id = group_id(&app, 0);
        app.ui.file_drop_hover_target = Some(target_group_id.clone());

        assert!(!app.consume_dropped_files_into_group(target_group_id));

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
}
