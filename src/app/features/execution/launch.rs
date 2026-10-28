use crate::app::features::execution::store::{RunningAppInstancesLookup, RunningAppSettingsUpdate};
use crate::app::features::execution::{
    ensure_package_owner_claim, is_excluded_installed_auto_process, InstalledPackageTrackingState,
    MonitorWake, RuntimeRegistry,
};
use crate::app::features::rules::RulesContext;
use crate::app::features::system_tweaks::tweaks::scheduling::{fields_for_rule, SchedulingHandoff};
use crate::app::models::{
    AppRuntimeKey, AppStateStorage, AppToRun, LaunchTarget, LogManager, ProcessInstanceToken,
};
use crate::app::shared::ids::{GroupId, RuleId};
use os_api::gaming::{ProcessLaunch, ProcessSettingsFields};
use os_api::{InstalledPackageRuntimeInfo, PriorityClass, ProcessSettingsApplyOutcome, OS};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Clone, Default)]
struct LaunchProcessSnapshot {
    children_of: HashMap<u32, Vec<u32>>,
    names: HashMap<u32, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PostLaunchCorrectionOutcome {
    seed_pids: Vec<u32>,
    managed_pids: Vec<u32>,
    no_identity_package_pids: Vec<u32>,
    new_managed_pids_added: usize,
    saw_identity_seed: bool,
    instance_tokens: HashMap<u32, ProcessInstanceToken>,
}

struct PostLaunchCorrectionRequest {
    source: AppToRun,
    wake: Option<MonitorWake>,
    running_apps: Arc<TokioRwLock<crate::app::models::RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    app_key: AppRuntimeKey,
    initial_pid: Option<u32>,
    initial_token: Option<ProcessInstanceToken>,
    group_id: GroupId,
    rule_id: RuleId,
    group_cores: Vec<usize>,
    priority: PriorityClass,
    expected_aumid: String,
    installed_package_info: Option<InstalledPackageRuntimeInfo>,
    prelaunch_package_pids: HashSet<u32>,
    manage_descendants: bool,
    fields: ProcessSettingsFields,
    scheduling: Arc<SchedulingHandoff>,
    control: Arc<super::control::Authority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchDispatchOutcome {
    Accepted,
    Rejected(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppRowAction {
    Run,
    Fix,
    Focus,
}

pub(crate) struct AppRowActionRequest {
    pub group_index: usize,
    pub program_index: usize,
    pub app: AppToRun,
    pub action: AppRowAction,
}

trait LaunchOs {
    fn launch_known_launcher(&self, app: &AppToRun) -> Result<(), String> {
        self.launch_ordinary(
            app.bin_path().ok_or("Missing application executable")?,
            &app.args,
        )
    }
    fn activate_known_client(&self, _app: &AppToRun) -> Result<(), String> {
        Err("Client activation unavailable".into())
    }

    fn launch_ordinary(&self, _path: &Path, _args: &[String]) -> Result<(), String> {
        Err("Ordinary application launch unavailable".into())
    }
    fn known_launch_valid(
        &self,
        _app: &AppToRun,
        _scope: &crate::app::features::rules::known_apps::Scope,
    ) -> bool {
        true
    }
    fn known_instance_role(
        &self,
        _pid: u32,
        _token: u64,
        scope: &crate::app::features::rules::known_apps::Scope,
        image: &Path,
    ) -> Option<crate::app::features::rules::known_apps::MemberRole> {
        scope.witness_role_under_root(&scope.root, image)
    }

    fn launch_steam(&self, _app_id: u32) -> Result<(), String> {
        Err("Steam launch unavailable".into())
    }
    fn get_process_instance_token(&self, pid: u32) -> Result<ProcessInstanceToken, String>;
    fn current_logon_instance(&self, pid: u32, token: ProcessInstanceToken) -> bool;
    fn apply_process_fields_if_instance(
        &self,
        pid: u32,
        expected_instance_token: ProcessInstanceToken,
        mask: usize,
        priority: PriorityClass,
        fields: ProcessSettingsFields,
    ) -> Result<ProcessSettingsApplyOutcome, String>;
    fn focus_window_by_pid(&self, pid: u32) -> bool;
    fn run_with_fields(
        &self,
        bin_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
        fields: ProcessSettingsFields,
    ) -> Result<ProcessLaunch, String>;
    fn activate_application(&self, aumid: &str) -> Result<u32, String>;
    fn start_post_launch_correction(&self, request: PostLaunchCorrectionRequest) {
        spawn_post_launch_correction(request);
    }
    fn snapshot_process_tree(&self) -> Result<LaunchProcessSnapshot, String>;
    fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String>;
    fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String>;
    fn resolve_installed_package_runtime_info(
        &self,
        aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String>;
}

#[cfg(test)]
use super::steam_pending::finish_steam_launch;
#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) use super::steam_pending::{begin_launch, cancel_launch, identity_for_app};
pub(crate) use super::steam_pending::{
    begin_steam_launch, cancel_steam_launch, steam_launch_pending, SteamLaunchBegin,
};
struct RealLaunchOs;

#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) struct InstalledActivationRequest<'a> {
    pub source: super::control::SourceRequest<'a>,
    pub wake: Option<MonitorWake>,
    pub running_apps: Arc<TokioRwLock<crate::app::models::RunningApps>>,
    pub installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    pub scheduling: Arc<SchedulingHandoff>,
    pub control: Arc<super::control::Authority>,
}

#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) fn activate_installed_with_correction(
    request: InstalledActivationRequest<'_>,
    activate: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    activate_installed_with_os(request, activate, &RealLaunchOs)
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn activate_installed_with_os(
    request: InstalledActivationRequest<'_>,
    activate: impl FnOnce(&str) -> Result<(), String>,
    os: &impl LaunchOs,
) -> Result<(), String> {
    let source = request.source;
    let aumid = source
        .app
        .installed_aumid()
        .ok_or("Expected an installed application")?
        .to_owned();
    let mut correction = PostLaunchCorrectionRequest {
        source: source.app.clone(),
        wake: request.wake,
        running_apps: request.running_apps,
        installed_package_tracking: request.installed_package_tracking,
        app_key: source.key.clone(),
        initial_pid: None,
        initial_token: None,
        group_id: source.group.clone(),
        rule_id: source.rule.clone(),
        group_cores: (0..usize::BITS as usize)
            .filter(|cpu| source.mask & (1usize << cpu) != 0)
            .collect(),
        priority: source.app.priority,
        expected_aumid: aumid,
        installed_package_info: None,
        prelaunch_package_pids: HashSet::new(),
        manage_descendants: source.app.manage_descendants,
        fields: source.fields,
        scheduling: request.scheduling,
        control: request.control,
    };
    correction
        .control
        .with_source(correction_source(&correction)?, |_| Ok(()))?;
    if let Some((info, pids)) = super::resolve_installed_package_runtime_info_cached(
        &correction.installed_package_tracking,
        &correction.expected_aumid,
        |aumid| os.resolve_installed_package_runtime_info(aumid),
    )
    .ok()
    .and_then(|info| {
        collect_package_local_pids_from_live_snapshot(os, &info.install_root)
            .ok()
            .map(|pids| (info, pids))
    }) {
        correction.installed_package_info = Some(info);
        correction.prelaunch_package_pids = pids;
    }
    correction
        .control
        .with_source(correction_source(&correction)?, |_| {
            activate(&correction.expected_aumid)
        })?;
    os.start_post_launch_correction(correction);
    Ok(())
}

impl LaunchOs for RealLaunchOs {
    fn launch_known_launcher(&self, app: &AppToRun) -> Result<(), String> {
        crate::app::features::rules::known_apps::launch_ordinary(app)
    }
    fn activate_known_client(&self, app: &AppToRun) -> Result<(), String> {
        crate::app::features::rules::known_apps::activate_client(app)
    }

    fn launch_ordinary(&self, path: &Path, args: &[String]) -> Result<(), String> {
        os_api::app_identity::launch_ordinary(path, args)
    }
    fn known_launch_valid(
        &self,
        app: &AppToRun,
        scope: &crate::app::features::rules::known_apps::Scope,
    ) -> bool {
        scope.qualify_launch(app)
    }
    fn known_instance_role(
        &self,
        pid: u32,
        token: u64,
        scope: &crate::app::features::rules::known_apps::Scope,
        image: &Path,
    ) -> Option<crate::app::features::rules::known_apps::MemberRole> {
        let (root, image) = os_api::app_identity::canonical_scope(&scope.root, image)?;
        let role = scope.witness_role_under_root(&root, &image)?;
        os_api::app_identity::verify_current_instance(pid, token).then_some(role)
    }

    fn launch_steam(&self, app_id: u32) -> Result<(), String> {
        os_api::steam::launch(app_id)
    }
    fn get_process_instance_token(&self, pid: u32) -> Result<ProcessInstanceToken, String> {
        OS::get_process_instance_token(pid)
    }

    fn current_logon_instance(&self, pid: u32, token: ProcessInstanceToken) -> bool {
        os_api::app_identity::verify_current_instance(pid, token)
    }

    fn apply_process_fields_if_instance(
        &self,
        pid: u32,
        expected_instance_token: ProcessInstanceToken,
        mask: usize,
        priority: PriorityClass,
        fields: ProcessSettingsFields,
    ) -> Result<ProcessSettingsApplyOutcome, String> {
        OS::apply_process_fields_if_instance(
            pid,
            expected_instance_token,
            mask,
            priority,
            fields,
            true,
        )
    }

    fn focus_window_by_pid(&self, pid: u32) -> bool {
        OS::focus_window_by_pid(pid)
    }

    fn run_with_fields(
        &self,
        bin_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
        fields: ProcessSettingsFields,
    ) -> Result<ProcessLaunch, String> {
        OS::run_with_fields(bin_path, args, cores, priority, fields)
    }

    fn activate_application(&self, aumid: &str) -> Result<u32, String> {
        OS::activate_application(aumid)
    }

    fn snapshot_process_tree(&self) -> Result<LaunchProcessSnapshot, String> {
        OS::snapshot_process_tree().map(|tree| LaunchProcessSnapshot {
            children_of: tree.children_of,
            names: tree.names,
        })
    }

    fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String> {
        OS::get_process_image_path(pid)
    }

    fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String> {
        OS::get_process_app_user_model_id(pid)
    }

    fn resolve_installed_package_runtime_info(
        &self,
        aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String> {
        OS::resolve_installed_package_runtime_info(aumid)
    }
}

pub(crate) fn collect_autorun_items(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
) -> Vec<(usize, usize, AppToRun)> {
    let state = persistent_state.read().unwrap();
    state
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g_i, group)| {
            group
                .programs
                .iter()
                .enumerate()
                .filter_map(move |(p_i, app)| {
                    if app.autorun {
                        Some((g_i, p_i, app.clone()))
                    } else {
                        None
                    }
                })
        })
        .collect()
}

pub fn start_app_with_autorun(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
) {
    let os = RealLaunchOs;

    for (g_i, p_i, app_to_run) in collect_autorun_items(persistent_state) {
        run_app_with_affinity_sync_with_os(
            persistent_state,
            runtime,
            log_manager,
            g_i,
            p_i,
            app_to_run,
            &os,
        );
    }
}

pub fn run_app_with_affinity_sync(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_index: usize,
    prog_index: usize,
    app_to_run: AppToRun,
) -> LaunchDispatchOutcome {
    let os = RealLaunchOs;

    run_app_with_affinity_sync_with_os(
        persistent_state,
        runtime,
        log_manager,
        group_index,
        prog_index,
        app_to_run,
        &os,
    )
}

pub fn run_app_row_action(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &mut RuntimeRegistry,
    log_manager: &mut LogManager,
    request: AppRowActionRequest,
) -> LaunchDispatchOutcome {
    let os = RealLaunchOs;

    run_app_row_action_with_os(persistent_state, runtime, log_manager, request, &os)
}

fn run_app_row_action_with_os<O: LaunchOs>(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &mut RuntimeRegistry,
    log_manager: &mut LogManager,
    request: AppRowActionRequest,
    os: &O,
) -> LaunchDispatchOutcome {
    let AppRowActionRequest {
        group_index,
        program_index,
        app: app_to_run,
        action,
    } = request;

    let selected_rule = {
        let state = persistent_state.read().unwrap();
        runtime.control_handle().initialize(&state);
        RulesContext::from_storage(&state).rule_id_for_index(group_index, program_index)
    };
    let Some(selected_rule) = selected_rule else {
        return LaunchDispatchOutcome::Rejected("Saved rule is unavailable".into());
    };
    let selected_key = runtime.control_key(&selected_rule, &app_to_run);
    match action {
        AppRowAction::Run => run_app_with_affinity_sync_with_os(
            persistent_state,
            runtime,
            log_manager,
            group_index,
            program_index,
            app_to_run,
            os,
        ),
        AppRowAction::Focus => {
            focus_existing_app_with_key(runtime, log_manager, app_to_run, selected_key, os)
        }
        AppRowAction::Fix => {
            let (group_cores, fields) = {
                let state = persistent_state.read().unwrap();
                match state.groups.get(group_index) {
                    Some(group) => {
                        let rules = RulesContext::from_storage(&state);
                        let fields = rules
                            .group_id_for_index(group_index)
                            .zip(rules.rule_id_for_index(group_index, program_index))
                            .map(|(group, rule)| fields_for_rule(&state, &group, &rule))
                            .unwrap_or_default();
                        (group.cores.clone(), fields)
                    }
                    None => {
                        let message = format!("Error: Group index {group_index} not found");
                        log_manager.add_important_entry(message.clone());
                        return LaunchDispatchOutcome::Rejected(message);
                    }
                }
            };
            reapply_existing_app_settings(
                runtime,
                log_manager,
                app_to_run,
                (selected_rule, selected_key),
                &group_cores,
                fields,
                os,
            )
        }
    }
}

pub(crate) fn affinity_mask_from_cores(cores: &[usize]) -> Result<usize, String> {
    if cores.is_empty() {
        return Err("CPU affinity has no selected logical processors".to_string());
    }

    cores.iter().try_fold(0usize, |mask, &core| {
        if core >= usize::BITS as usize {
            return Err(format!(
                "logical processor index {core} exceeds the supported affinity-mask width ({})",
                usize::BITS
            ));
        }
        let bit = 1usize << core;
        Ok(mask | bit)
    })
}

fn reapply_existing_app_settings<O: LaunchOs>(
    runtime: &mut RuntimeRegistry,
    log_manager: &mut LogManager,
    app_to_run: AppToRun,
    identity: (RuleId, AppRuntimeKey),
    group_cores: &[usize],
    fields: ProcessSettingsFields,
    os: &O,
) -> LaunchDispatchOutcome {
    let (rule_id, app_key) = identity;
    let instances = match runtime.lookup_running_app_instances(&app_key) {
        RunningAppInstancesLookup::Found(instances) if !instances.is_empty() => instances,
        RunningAppInstancesLookup::Found(_) | RunningAppInstancesLookup::NotFound => {
            let message = format!(
                "Cannot fix {}: the app is not running",
                app_to_run.display()
            );
            log_manager.add_important_entry(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        }
        RunningAppInstancesLookup::Busy => {
            let message = format!(
                "Cannot fix {}: running app state is temporarily busy; try again",
                app_to_run.display()
            );
            log_manager.add_important_entry(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        }
    };
    let mask = match if fields.affinity {
        affinity_mask_from_cores(group_cores)
    } else {
        Ok(0)
    } {
        Ok(mask) => mask,
        Err(error) => {
            let message = format!("Cannot fix {}: {error}", app_to_run.display());
            log_manager.add_important_entry(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        }
    };
    let (failures, shared) = apply_settings_for_instances(
        os,
        &instances,
        &runtime.scheduling_handle(),
        RuleWriteContext {
            mask,
            priority: app_to_run.priority,
            requested: fields,
            control: &runtime.control_handle(),
            rule: &rule_id,
            key: &app_key,
        },
    );

    if !failures.is_empty() {
        let _ = runtime.mark_running_app_settings_mismatched(&app_key);
        let message = format!(
            "Failed to fix settings for {}: {}",
            app_to_run.display(),
            failures.join("; ")
        );
        log_manager.add_important_entry(message.clone());
        return LaunchDispatchOutcome::Rejected(message);
    }

    if shared {
        let _ = runtime.mark_running_app_settings_mismatched(&app_key);
        log_manager.add_entry(
            "Owned processes updated; shared processes retain their chosen controller".into(),
        );
        return LaunchDispatchOutcome::Accepted;
    }
    match runtime.mark_running_app_settings_matched(&app_key) {
        RunningAppSettingsUpdate::Updated => {
            log_manager.add_entry(format!("App settings reapplied: {}", app_to_run.display()));
            LaunchDispatchOutcome::Accepted
        }
        RunningAppSettingsUpdate::NotFound => {
            let message = format!(
                "Settings were applied for {}, but the app stopped before confirmation",
                app_to_run.display()
            );
            log_manager.add_important_entry(message.clone());
            LaunchDispatchOutcome::Rejected(message)
        }
        RunningAppSettingsUpdate::Busy => {
            let message = format!(
                "Settings were applied for {}, but status confirmation is temporarily busy; try again",
                app_to_run.display()
            );
            log_manager.add_important_entry(message.clone());
            LaunchDispatchOutcome::Rejected(message)
        }
    }
}

struct RuleWriteContext<'a> {
    mask: usize,
    priority: PriorityClass,
    requested: ProcessSettingsFields,
    control: &'a super::control::Authority,
    rule: &'a RuleId,
    key: &'a AppRuntimeKey,
}
fn apply_settings_for_instances<O: LaunchOs>(
    os: &O,
    instances: &[(u32, ProcessInstanceToken)],
    scheduling: &SchedulingHandoff,
    context: RuleWriteContext<'_>,
) -> (Vec<String>, bool) {
    let RuleWriteContext {
        mask,
        priority,
        requested,
        control,
        rule,
        key,
    } = context;
    let mut failures = Vec::new();
    let mut shared = false;

    for &(pid, instance_token) in instances {
        let image = os.get_process_image_path(pid).ok();
        let result = control.write(
            super::control::WriteRequest {
                rule,
                key,
                pid,
                token: instance_token,
                image: image.as_deref(),
                mask,
                priority,
                fields: requested,
                change: true,
            },
            |allowed| {
                if !allowed {
                    return Ok(());
                }
                let fields = scheduling
                    .select(pid, instance_token, requested, true)
                    .map_err(|error| format!("{error:?}"))?;
                os.apply_process_fields_if_instance(pid, instance_token, mask, priority, fields)
                    .map(|_| ())
            },
        );
        match result {
            Err(error) => failures.push(format!("PID {pid}: {error}")),
            Ok((_, false)) => shared = true,
            _ => {}
        }
    }

    (failures, shared)
}

fn known_focus_allowed<O: LaunchOs>(app: &AppToRun, os: &O, pid: u32, token: u64) -> bool {
    if !app.is_path_target() || app.builtin_process_binding.is_none() {
        return true;
    }
    let Ok(scope) = crate::app::features::rules::known_apps::project(app) else {
        return false;
    };
    os.get_process_image_path(pid)
        .ok()
        .is_some_and(|image| os.known_instance_role(pid, token, &scope, &image).is_some())
}

fn activate_owned_client<O: LaunchOs>(
    app: &AppToRun,
    instances: &[(u32, u64)],
    os: &O,
) -> Result<bool, String> {
    if app.builtin_process_binding.is_none() || !app.is_path_target() {
        return Ok(false);
    }
    let scope = crate::app::features::rules::known_apps::project(app)?;
    if !scope.can_activate_hidden() {
        return Ok(false);
    }
    let eligible = instances.iter().any(|&(pid, token)| {
        os.get_process_instance_token(pid).ok() == Some(token)
            && known_focus_allowed(app, os, pid, token)
    });
    if !eligible || !os.known_launch_valid(app, &scope) {
        return Ok(false);
    }
    os.activate_known_client(app).map(|()| true)
}

#[cfg(test)]
fn focus_existing_app<O: LaunchOs>(
    runtime: &RuntimeRegistry,
    log: &mut LogManager,
    app: AppToRun,
    os: &O,
) -> LaunchDispatchOutcome {
    let key = app.get_key();
    focus_existing_app_with_key(runtime, log, app, key, os)
}
fn focus_existing_app_with_key<O: LaunchOs>(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    app_to_run: AppToRun,
    app_key: AppRuntimeKey,
    os: &O,
) -> LaunchDispatchOutcome {
    let instances =
        match runtime.lookup_focus_instances(&app_key, app_to_run.steam_game().is_some()) {
            RunningAppInstancesLookup::Found(instances) if !instances.is_empty() => instances,
            RunningAppInstancesLookup::Found(_) | RunningAppInstancesLookup::NotFound => {
                let message = format!(
                    "Cannot focus {}: the app is not running",
                    app_to_run.display()
                );
                log_manager.add_important_entry(message.clone());
                return LaunchDispatchOutcome::Rejected(message);
            }
            RunningAppInstancesLookup::Busy => {
                let message = format!(
                    "Cannot focus {}: running app state is temporarily busy; try again",
                    app_to_run.display()
                );
                log_manager.add_important_entry(message.clone());
                return LaunchDispatchOutcome::Rejected(message);
            }
        };

    if instances
        .iter()
        .filter(|&&(pid, _)| {
            app_to_run.steam_game().is_none_or(|(_, selected)| {
                selected.is_none_or(|selected| {
                    os.get_process_image_path(pid).is_ok_and(|image| {
                        image
                            .to_string_lossy()
                            .eq_ignore_ascii_case(&selected.to_string_lossy())
                            || image
                                .canonicalize()
                                .ok()
                                .zip(selected.canonicalize().ok())
                                .is_some_and(|(a, b)| a == b)
                    })
                })
            })
        })
        .any(|&(pid, expected_token)| {
            os.get_process_instance_token(pid)
                .is_ok_and(|current_token| current_token == expected_token)
                && known_focus_allowed(&app_to_run, os, pid, expected_token)
                && os.focus_window_by_pid(pid)
        })
    {
        log_manager.add_entry(format!("App window focused: {}", app_to_run.display()));
        LaunchDispatchOutcome::Accepted
    } else {
        match activate_owned_client(&app_to_run, &instances, os) {
            Ok(true) => {
                log_manager.add_entry(format!("Client activation requested: {}", app_to_run.name));
                return LaunchDispatchOutcome::Accepted;
            }
            Err(error) => {
                log_manager.add_important_entry(error.clone());
                return LaunchDispatchOutcome::Rejected(error);
            }
            Ok(false) => {}
        }
        let message = format!(
            "Cannot focus {}: no application window was found",
            app_to_run.display()
        );
        log_manager.add_important_entry(message.clone());
        LaunchDispatchOutcome::Rejected(message)
    }
}

fn run_app_with_affinity_sync_with_os<O: LaunchOs>(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_index: usize,
    prog_index: usize,
    app_to_run: AppToRun,
    os: &O,
) -> LaunchDispatchOutcome {
    let group_cores = {
        let state = persistent_state.read().unwrap();
        match state.groups.get(group_index) {
            Some(group) => group.cores.clone(),
            None => {
                let message = format!("Error: Group index {group_index} not found");
                log_manager.add_important_sticky_once(message.clone());
                return LaunchDispatchOutcome::Rejected(message);
            }
        }
    };

    let (group_id, rule_id, fields) = {
        let state = persistent_state.read().unwrap();
        runtime.control_handle().initialize(&state);
        let rules = RulesContext::from_storage(&state);
        let Some(group_id) = rules.group_id_for_index(group_index) else {
            let message = format!("Error: Missing group id for group index {group_index}");
            log_manager.add_important_sticky_once(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        };
        let Some(rule_id) = rules.rule_id_for_index(group_index, prog_index) else {
            let message = format!(
                "Error: Missing rule id for rule index {prog_index} in group {group_index}"
            );
            log_manager.add_important_sticky_once(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        };
        let fields = fields_for_rule(&state, &group_id, &rule_id);
        (group_id, rule_id, fields)
    };

    run_launch_decision_with_fields(
        runtime,
        log_manager,
        group_id,
        rule_id,
        app_to_run,
        group_cores,
        fields,
        os,
    )
}

#[cfg(test)]
fn run_launch_decision<O: LaunchOs>(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_id: GroupId,
    rule_id: RuleId,
    app_to_run: AppToRun,
    group_cores: Vec<usize>,
    os: &O,
) -> LaunchDispatchOutcome {
    let fixture = AppStateStorage::test_fixture(|state| {
        state.groups = vec![crate::app::models::CoreGroup {
            name: "Fixture".into(),
            cores: group_cores.clone(),
            programs: vec![app_to_run.clone()],
            is_hidden: false,
            run_all_button: false,
        }];
        state.rule_identities = Some(crate::app::features::rules::PersistedRuleIdentities {
            groups: vec![crate::app::features::rules::PersistedGroupIdentity {
                id: group_id.clone(),
                rule_ids: vec![rule_id.clone()],
            }],
            next_group_id: 0,
            next_rule_id: 0,
        });
    });
    runtime.control_handle().configure(&fixture);
    run_launch_decision_with_fields(
        runtime,
        log_manager,
        group_id,
        rule_id,
        app_to_run,
        group_cores,
        ProcessSettingsFields::default(),
        os,
    )
}
#[allow(clippy::too_many_arguments)] // Adds only field selection to the existing launch boundary.
fn run_launch_decision_with_fields<O: LaunchOs>(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_id: GroupId,
    rule_id: RuleId,
    app_to_run: AppToRun,
    group_cores: Vec<usize>,
    fields: ProcessSettingsFields,
    os: &O,
) -> LaunchDispatchOutcome {
    let app_key = runtime.control_key(&rule_id, &app_to_run);
    let mask = match if fields.affinity {
        affinity_mask_from_cores(&group_cores)
    } else {
        Ok(0)
    } {
        Ok(mask) => mask,
        Err(error) => {
            let message = format!("Launch skipped for {}: {error}", app_to_run.display());
            log_manager.add_important_entry(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        }
    };

    let known_scope = if app_to_run.is_path_target() && app_to_run.builtin_process_binding.is_some()
    {
        match crate::app::features::rules::known_apps::project(&app_to_run) {
            Ok(scope) if os.known_launch_valid(&app_to_run, &scope) => Some(scope),
            _ => {
                let error="Built-in application definition or installation is unavailable; re-add this application to refresh its defaults".to_string();
                log_manager.add_important_entry(error.clone());
                return LaunchDispatchOutcome::Rejected(error);
            }
        }
    } else {
        None
    };
    let mut existing = runtime.lookup_running_app_instances(&app_key);
    if let (Some(scope), RunningAppInstancesLookup::Found(instances)) = (&known_scope, &existing) {
        let mut evidence = instances.clone();
        if let RunningAppInstancesLookup::Found(primary) =
            runtime.lookup_focus_instances(&app_key, true)
        {
            evidence.extend(primary);
        }
        let mut main = false;
        let mut uncertain = false;
        for (pid, token) in evidence {
            match (
                os.get_process_instance_token(pid),
                os.get_process_image_path(pid),
            ) {
                (Ok(current), Ok(image)) if current == token => {
                    main |= os.known_instance_role(pid, token, scope, &image)
                        == Some(crate::app::features::rules::known_apps::MemberRole::Main);
                }
                (Err(_), _) | (_, Err(_)) => uncertain = true,
                _ => {}
            }
        }
        if !main {
            if uncertain {
                return LaunchDispatchOutcome::Rejected(
                    "Application identity is temporarily unavailable; try again".into(),
                );
            }
            existing = RunningAppInstancesLookup::NotFound;
        }
    }
    match existing {
        RunningAppInstancesLookup::Found(instances) if !instances.is_empty() => {
            let (failures, _shared) = apply_settings_for_instances(
                os,
                &instances,
                &runtime.scheduling_handle(),
                RuleWriteContext {
                    mask,
                    priority: app_to_run.priority,
                    requested: fields,
                    control: &runtime.control_handle(),
                    rule: &rule_id,
                    key: &app_key,
                },
            );
            if !failures.is_empty() {
                match runtime.try_mark_running_app_settings_mismatched(&app_key) {
                    RunningAppSettingsUpdate::Updated => {}
                    RunningAppSettingsUpdate::NotFound => log_manager.add_important_entry(format!(
                        "Failed to record settings mismatch for {} because its tracked entry disappeared",
                        app_to_run.display()
                    )),
                    RunningAppSettingsUpdate::Busy => log_manager.add_important_entry(format!(
                        "Failed to record settings mismatch for {} because running app state is temporarily busy",
                        app_to_run.display()
                    )),
                }

                let message = format!(
                    "Failed to reapply settings for {}: {}",
                    app_to_run.display(),
                    failures.join("; ")
                );
                log_manager.add_important_entry(message.clone());
                return LaunchDispatchOutcome::Rejected(message);
            }

            let focus_instances =
                runtime.lookup_focus_instances(&app_key, app_to_run.steam_game().is_some());
            let focus_instances = match focus_instances {
                RunningAppInstancesLookup::Found(v) => v,
                _ => Vec::new(),
            };
            let was_focused = focus_instances.iter().any(|&(pid, expected_token)| {
                os.get_process_instance_token(pid)
                    .is_ok_and(|current_token| current_token == expected_token)
                    && known_focus_allowed(&app_to_run, os, pid, expected_token)
                    && os.focus_window_by_pid(pid)
            });
            if was_focused {
                log_manager.add_entry(format!(
                    "App already running: {}, settings reapplied and window focused",
                    app_to_run.display()
                ));
                return LaunchDispatchOutcome::Accepted;
            }

            match activate_owned_client(&app_to_run, &focus_instances, os) {
                Ok(true) => {
                    log_manager
                        .add_entry(format!("Client activation requested: {}", app_to_run.name));
                    return LaunchDispatchOutcome::Accepted;
                }
                Err(error) => {
                    log_manager.add_important_entry(error.clone());
                    return LaunchDispatchOutcome::Rejected(error);
                }
                Ok(false) => {}
            }
            log_manager.add_entry(format!(
                "App already running: {}, settings reapplied but no window found to focus",
                app_to_run.display()
            ));
            return LaunchDispatchOutcome::Accepted;
        }
        RunningAppInstancesLookup::Found(_) => {
            let message = format!(
                "Launch skipped for {}: tracked process identity is not available yet; try again",
                app_to_run.display()
            );
            log_manager.add_important_entry(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        }
        RunningAppInstancesLookup::NotFound => {}
        RunningAppInstancesLookup::Busy => {
            let message = format!(
                "Launch skipped for {}: running app state is temporarily busy; try again",
                app_to_run.display()
            );
            log_manager.add_important_entry(message.clone());
            return LaunchDispatchOutcome::Rejected(message);
        }
    }

    if let Some((app_id, _)) = app_to_run.steam_game() {
        let generation = match begin_steam_launch(&rule_id, app_id) {
            SteamLaunchBegin::AlreadyPending => return LaunchDispatchOutcome::Accepted,
            SteamLaunchBegin::Started(generation) => generation,
            SteamLaunchBegin::Busy => {
                return LaunchDispatchOutcome::Rejected(
                    "Too many pending Steam launches; try again later".into(),
                )
            }
        };
        return match os.launch_steam(app_id) {
            Ok(()) => {
                log_manager.add_entry(format!("Steam launch requested for {}. Waiting for the game process; use Choose game process in its settings if detection is unavailable.",app_to_run.name));
                LaunchDispatchOutcome::Accepted
            }
            Err(error) => {
                cancel_steam_launch(&rule_id, app_id, generation);
                log_manager.add_important_entry(format!("Steam launch failed: {error}"));
                LaunchDispatchOutcome::Rejected(error)
            }
        };
    }
    let known_intent = if known_scope.is_some() && !app_to_run.additional_processes.is_empty() {
        let identity =
            super::steam_pending::identity_for_app(&app_to_run).expect("validated known scope");
        match super::steam_pending::begin_launch(&rule_id, identity.clone()) {
            SteamLaunchBegin::AlreadyPending => return LaunchDispatchOutcome::Accepted,
            SteamLaunchBegin::Started(generation) => Some((identity, generation)),
            SteamLaunchBegin::Busy => {
                return LaunchDispatchOutcome::Rejected(
                    "Too many pending application launches; try again later".into(),
                )
            }
        }
    } else {
        None
    };
    if known_scope
        .as_ref()
        .is_some_and(|scope| scope.is_launcher())
    {
        let result = os.launch_known_launcher(&app_to_run);
        return match result {
            Ok(()) => {
                log_manager.add_entry(format!(
                    "{} launch requested. Waiting for its application processes.",
                    app_to_run.name
                ));
                LaunchDispatchOutcome::Accepted
            }
            Err(error) => {
                if let Some((identity, generation)) = known_intent {
                    super::steam_pending::cancel_launch(&rule_id, identity, generation);
                }
                log_manager.add_important_entry(error.clone());
                LaunchDispatchOutcome::Rejected(error)
            }
        };
    }
    let label = match &app_to_run.launch_target {
        LaunchTarget::Path { bin_path, .. } => bin_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| bin_path.display().to_string()),
        LaunchTarget::Installed { .. } | LaunchTarget::Steam { .. } => app_to_run.name.clone(),
    };
    let display = app_to_run.display();
    let priority = app_to_run.priority;
    let (installed_package_info, prelaunch_package_pids) =
        if let LaunchTarget::Installed { aumid } = &app_to_run.launch_target {
            match runtime
                .resolve_installed_package_runtime_info_with(aumid, |aumid| {
                    os.resolve_installed_package_runtime_info(aumid)
                })
                .ok()
                .and_then(|info| {
                    collect_package_local_pids_from_live_snapshot(os, &info.install_root)
                        .ok()
                        .map(|pids| (info, pids))
                }) {
                Some((info, pids)) => (Some(info), pids),
                None => (None, HashSet::new()),
            }
        } else {
            (None, HashSet::new())
        };

    log_manager.add_entry(format!("Starting '{}', app: {}", label, display));

    let mut launched_fields = fields;
    let launch_result = match &app_to_run.launch_target {
        LaunchTarget::Path { bin_path, .. } => runtime
            .control_handle()
            .launch(&rule_id, &app_key, bin_path, |owner| {
                launched_fields = owner.fields;
                let cores: Vec<_> = (0..usize::BITS as usize)
                    .filter(|cpu| owner.mask & (1usize << cpu) != 0)
                    .collect();
                os.run_with_fields(
                    bin_path.clone(),
                    app_to_run.args.clone(),
                    &cores,
                    if owner.fields.priority {
                        owner.app.priority
                    } else {
                        PriorityClass::Normal
                    },
                    owner.fields,
                )
            })
            .map(|launch| (launch.pid, Some(launch.creation_token))),
        // Successful activation is still accepted if its initial PID cannot be inspected.
        // The existing correction worker can discover a verified process later.
        LaunchTarget::Installed { aumid } => os.activate_application(aumid).map(|pid| (pid, None)),
        LaunchTarget::Steam { .. } => unreachable!(),
    };

    match launch_result {
        Ok((pid, created_token)) => {
            let is_installed = matches!(app_to_run.launch_target, LaunchTarget::Installed { .. });
            let launch_pid_auto_managed =
                !is_installed || installed_launch_pid_auto_managed(os, pid);
            let launch_instance_token = os
                .get_process_instance_token(pid)
                .ok()
                .filter(|token| created_token.is_none_or(|created| *token == created));
            if !is_installed {
                if let Some(token) = launch_instance_token {
                    if let Err(error) =
                        runtime
                            .scheduling_handle()
                            .started(pid, token, launched_fields)
                    {
                        log_manager.add_important_entry(format!(
                            "Process scheduling baseline unavailable: {}",
                            error.detail().unwrap_or(error.operation)
                        ));
                    }
                }
            }

            if is_installed && launch_pid_auto_managed {
                if let Some(instance_token) = launch_instance_token {
                    let _ = apply_initial_process(
                        os,
                        &runtime.control_handle(),
                        &runtime.scheduling_handle(),
                        super::control::SourceRequest {
                            group: &group_id,
                            rule: &rule_id,
                            key: &app_key,
                            app: &app_to_run,
                            mask,
                            fields,
                        },
                        pid,
                        instance_token,
                    );
                }
            }

            if launch_pid_auto_managed {
                if let Some(instance_token) = launch_instance_token {
                    record_started_pid(
                        runtime,
                        log_manager,
                        &app_key,
                        pid,
                        instance_token,
                        group_id.clone(),
                        rule_id.clone(),
                    );
                } else {
                    log_manager.add_important_entry(format!(
                        "App started with PID {pid}, but its process identity could not be verified; waiting for rediscovery"
                    ));
                }
            } else {
                log_manager.add_entry(format!(
                    "Installed app activation PID {pid} is a Windows background host; waiting for app processes"
                ));
            }

            if let LaunchTarget::Installed { aumid } = &app_to_run.launch_target {
                os.start_post_launch_correction(PostLaunchCorrectionRequest {
                    source: app_to_run.clone(),
                    wake: runtime.wake_handle(),
                    running_apps: runtime.running_apps_handle(),
                    installed_package_tracking: runtime.installed_package_tracking_handle(),
                    app_key,
                    initial_pid: Some(pid),
                    initial_token: launch_instance_token,
                    group_id,
                    rule_id,
                    group_cores,
                    priority,
                    expected_aumid: aumid.clone(),
                    installed_package_info,
                    prelaunch_package_pids,
                    manage_descendants: app_to_run.manage_descendants,
                    fields,
                    scheduling: runtime.scheduling_handle(),
                    control: runtime.control_handle(),
                });
            }
            LaunchDispatchOutcome::Accepted
        }
        Err(e) => {
            if let Some((identity, generation)) = known_intent {
                super::steam_pending::cancel_launch(&rule_id, identity, generation);
            }
            log_manager.add_important_sticky_once(format!("ERROR: {e}"));
            LaunchDispatchOutcome::Rejected(e)
        }
    }
}

fn record_started_pid(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    app_key: &AppRuntimeKey,
    pid: u32,
    instance_token: ProcessInstanceToken,
    group_id: GroupId,
    rule_id: RuleId,
) {
    let is_new_app = !runtime.contains_app(app_key);

    if is_new_app {
        let added =
            runtime.add_running_app_with_token(app_key, pid, instance_token, group_id, rule_id);
        if added {
            log_manager.add_entry(format!("App started with PID: {pid}"));
        } else {
            log_manager.add_entry(format!(
                "App started with PID: {pid} but couldn't be tracked (lock busy)"
            ));
        }
    } else {
        let _ = runtime.add_pid_to_existing_app_with_token(app_key, pid, instance_token);
        log_manager.add_entry(format!(
            "New instance of existing app started with PID: {pid}"
        ));
    }
}

struct PostLaunchPublication {
    changed: bool,
    helpers: Vec<u32>,
}
fn correction_source(
    request: &PostLaunchCorrectionRequest,
) -> Result<super::control::SourceRequest<'_>, String> {
    Ok(super::control::SourceRequest {
        group: &request.group_id,
        rule: &request.rule_id,
        key: &request.app_key,
        app: &request.source,
        mask: if request.fields.affinity {
            affinity_mask_from_cores(&request.group_cores)?
        } else {
            0
        },
        fields: request.fields,
    })
}
fn complete_post_launch_correction<O: LaunchOs>(
    request: &PostLaunchCorrectionRequest,
    outcome: &PostLaunchCorrectionOutcome,
    os: &O,
) -> Result<PostLaunchPublication, String> {
    let mut apps = request
        .running_apps
        .try_write()
        .map_err(|_| "Running app state is busy")?;
    let publication = request
        .control
        .with_source(correction_source(request)?, |spec| {
            let live_tokens: HashMap<_, _> = outcome
                .instance_tokens
                .iter()
                .filter(|(pid, token)| os.get_process_instance_token(**pid).ok() == Some(**token))
                .map(|(&pid, &token)| (pid, token))
                .collect();
            let key = request
                .control
                .member(&apps, &spec.key)
                .map(|(key, _)| key.clone())
                .unwrap_or_else(|| spec.key.clone());
            let mut changed = false;
            let mut helpers = Vec::new();
            apps.mutate_membership(|apps| {
                if !apps.apps.contains_key(&key) {
                    if let Some(&pid) = outcome
                        .managed_pids
                        .iter()
                        .find(|pid| live_tokens.contains_key(pid))
                    {
                        apps.add_app(&key, pid, spec.group.clone(), spec.rule.clone());
                        changed = true;
                    }
                }
                if let Some(app) = apps.apps.get_mut(&key) {
                    changed |= app.group_id != spec.group || app.rule_id != spec.rule;
                    app.group_id = spec.group.clone();
                    app.rule_id = spec.rule.clone();
                    for &pid in &outcome.managed_pids {
                        if let Some(&token) = live_tokens.get(&pid) {
                            if !app.pids.contains(&pid) {
                                app.pids.push(pid);
                                changed = true;
                            }
                            changed |= app.pid_instance_tokens.insert(pid, token) != Some(token);
                        }
                    }
                }
                if apps.apps.contains_key(&key) {
                    if let Some(package) = &request.installed_package_info {
                        if ensure_package_owner_claim(
                            &mut request.installed_package_tracking.write().unwrap(),
                            apps,
                            &package.package_family_name,
                            &key,
                        ) {
                            if let Some(app) = apps.apps.get_mut(&key) {
                                for &pid in &outcome.no_identity_package_pids {
                                    if !app.pids.contains(&pid) {
                                        if let Some(&token) = live_tokens.get(&pid) {
                                            app.pids.push(pid);
                                            app.pid_instance_tokens.insert(pid, token);
                                            helpers.push(pid);
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            });
            Ok(PostLaunchPublication { changed, helpers })
        })?;
    drop(apps);
    for &pid in &publication.helpers {
        if let Some(&token) = outcome.instance_tokens.get(&pid) {
            let _ = apply_initial_process(
                os,
                &request.control,
                &request.scheduling,
                correction_source(request)?,
                pid,
                token,
            );
        }
    }
    Ok(publication)
}

fn spawn_post_launch_correction(request: PostLaunchCorrectionRequest) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    handle.spawn(async move {
        let os = RealLaunchOs;
        let mut seed_tokens: HashMap<_, _> = request
            .initial_pid
            .zip(request.initial_token)
            .into_iter()
            .collect();
        let mut seed_pids: Vec<_> = seed_tokens.keys().copied().collect();
        let mut stable_polls = 0usize;
        let mut saw_identity_seed = false;

        for delay_ms in [0u64, 100, 250, 500, 1000, 2000, 3500] {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            let Ok(source) = correction_source(&request) else {
                break;
            };
            if request.control.with_source(source, |_| Ok(())).is_err() {
                break;
            }
            let outcome = post_launch_correction_poll_with_fields(
                &os,
                &request.expected_aumid,
                &mut seed_pids,
                &mut seed_tokens,
                &request.group_cores,
                request.priority,
                request.installed_package_info.as_ref(),
                &request.prelaunch_package_pids,
                request.manage_descendants,
                request.fields,
                &request.scheduling,
                Some((&request.control, source)),
                request.initial_token.is_none(),
            );

            if let Ok(outcome) = outcome {
                if outcome.saw_identity_seed {
                    saw_identity_seed = true;
                }

                let Ok(publication) = complete_post_launch_correction(&request, &outcome, &os)
                else {
                    continue;
                };
                let runtime_changed = publication.changed;
                let attached_no_identity_pids = publication.helpers;
                let newly_attached_package_pids = attached_no_identity_pids.len();

                // The writer guard is gone before the UI is woken.
                if runtime_changed {
                    if let Some(wake) = &request.wake {
                        wake();
                    }
                }

                seed_pids = outcome.seed_pids;
                for &pid in &attached_no_identity_pids {
                    if let Some(&token) = outcome.instance_tokens.get(&pid) {
                        seed_tokens.insert(pid, token);
                    }
                }
                seed_pids.extend(attached_no_identity_pids);

                if outcome.new_managed_pids_added + newly_attached_package_pids == 0 {
                    stable_polls += 1;
                } else {
                    stable_polls = 0;
                }

                if saw_identity_seed && !outcome.managed_pids.is_empty() && stable_polls >= 2 {
                    break;
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)] // The test seam mirrors the bounded post-launch request.
#[cfg(test)]
fn post_launch_correction_poll_with_os<O: LaunchOs>(
    os: &O,
    expected_aumid: &str,
    seed_pids: &mut Vec<u32>,
    group_cores: &[usize],
    priority: PriorityClass,
    installed_package_info: Option<&InstalledPackageRuntimeInfo>,
    prelaunch_package_pids: &HashSet<u32>,
    manage_descendants: bool,
) -> Result<PostLaunchCorrectionOutcome, String> {
    let mut seed_tokens = seed_pids
        .iter()
        .filter_map(|&pid| {
            os.get_process_instance_token(pid)
                .ok()
                .map(|token| (pid, token))
        })
        .collect();
    post_launch_correction_poll_with_fields(
        os,
        expected_aumid,
        seed_pids,
        &mut seed_tokens,
        group_cores,
        priority,
        installed_package_info,
        prelaunch_package_pids,
        manage_descendants,
        ProcessSettingsFields::default(),
        &SchedulingHandoff::default(),
        None,
        false,
    )
}
fn apply_initial_process<O: LaunchOs>(
    os: &O,
    control: &super::control::Authority,
    scheduling: &SchedulingHandoff,
    source: super::control::SourceRequest<'_>,
    pid: u32,
    token: u64,
) -> Result<(), String> {
    let image = os.get_process_image_path(pid).ok();
    control.initial_with_source(
        source,
        super::control::WriteRequest {
            rule: source.rule,
            key: source.key,
            pid,
            token,
            image: image.as_deref(),
            mask: source.mask,
            priority: source.app.priority,
            fields: source.fields,
            change: true,
        },
        |owner| {
            let selected = scheduling
                .select(pid, token, owner.fields, true)
                .map_err(|error| format!("{error:?}"))?;
            os.apply_process_fields_if_instance(
                pid,
                token,
                owner.mask,
                owner.app.priority,
                selected,
            )
            .map(|_| ())
        },
    )
}
#[allow(clippy::too_many_arguments)] // Same bounded post-launch request plus field handoff metadata.
fn post_launch_correction_poll_with_fields<O: LaunchOs>(
    os: &O,
    expected_aumid: &str,
    seed_pids: &mut Vec<u32>,
    seed_tokens: &mut HashMap<u32, ProcessInstanceToken>,
    group_cores: &[usize],
    priority: PriorityClass,
    installed_package_info: Option<&InstalledPackageRuntimeInfo>,
    prelaunch_package_pids: &HashSet<u32>,
    manage_descendants: bool,
    fields: ProcessSettingsFields,
    scheduling: &SchedulingHandoff,
    authority: Option<(
        &super::control::Authority,
        super::control::SourceRequest<'_>,
    )>,
    acknowledged: bool,
) -> Result<PostLaunchCorrectionOutcome, String> {
    let snapshot = os.snapshot_process_tree()?;
    let before: HashSet<u32> = seed_pids.iter().copied().collect();

    // An acknowledgement or uninspectable activation carries no verified
    // lifetime. Discover exact same-logon AUMID seeds in the existing finite poll.
    if acknowledged {
        seed_pids.clear();
        seed_tokens.clear();
        for &pid in snapshot.names.keys() {
            let Ok(token) = os.get_process_instance_token(pid) else {
                continue;
            };
            if os
                .get_process_app_user_model_id(pid)
                .ok()
                .flatten()
                .is_some_and(|aumid| aumid.eq_ignore_ascii_case(expected_aumid))
                && os.current_logon_instance(pid, token)
            {
                seed_tokens.insert(pid, token);
                seed_pids.push(pid);
                if seed_pids.len() == 4096 {
                    break;
                }
            }
        }
    } else {
        // Retaining a PID from an earlier poll never grants a new lifetime.
        seed_pids.retain(|pid| {
            seed_tokens
                .get(pid)
                .is_some_and(|token| os.get_process_instance_token(*pid).ok() == Some(*token))
        });
    }

    if manage_descendants {
        extend_with_descendants(&snapshot, seed_pids);
        for &pid in seed_pids.iter() {
            if let std::collections::hash_map::Entry::Vacant(entry) = seed_tokens.entry(pid) {
                if let Ok(token) = os.get_process_instance_token(pid) {
                    entry.insert(token);
                }
            }
        }
    }
    let mut no_identity_package_pids = Vec::new();
    let mut package_tokens = HashMap::new();

    if let Some(package_info) = installed_package_info {
        let package_candidates = collect_package_local_pid_candidates_from_snapshot(
            os,
            &snapshot,
            &package_info.install_root,
            expected_aumid,
            prelaunch_package_pids,
            &before,
        )?;

        for pid in package_candidates.same_aumid_pids {
            if let Some(&token) = package_candidates.instance_tokens.get(&pid) {
                seed_tokens.entry(pid).or_insert(token);
            }
            if !seed_pids.contains(&pid) {
                seed_pids.push(pid);
            }
        }
        no_identity_package_pids = package_candidates.no_identity_pids;
        package_tokens = package_candidates.instance_tokens;
        retain_auto_managed_installed_pids(&snapshot, &mut no_identity_package_pids);
    }

    let mask = if fields.affinity {
        affinity_mask_from_cores(group_cores)?
    } else {
        0
    };
    if authority.is_some_and(|(_, source)| {
        source.mask != mask || source.app.priority != priority || source.fields != fields
    }) {
        return Err("Initial correction settings do not match its source".into());
    }
    let mut saw_identity_seed = false;
    let mut managed_pids = seed_pids.clone();
    retain_auto_managed_installed_pids(&snapshot, &mut managed_pids);
    let mut instance_tokens = HashMap::new();

    managed_pids.retain(|&pid| {
        let Ok(instance_token) = os.get_process_instance_token(pid) else {
            return false;
        };
        if seed_tokens.get(&pid) != Some(&instance_token)
            || package_tokens
                .get(&pid)
                .is_some_and(|token| *token != instance_token)
            || (acknowledged && !os.current_logon_instance(pid, instance_token))
        {
            return false;
        }
        let result = if let Some((control, source)) = authority {
            apply_initial_process(os, control, scheduling, source, pid, instance_token)
        } else {
            #[cfg(test)]
            {
                let selected = scheduling
                    .select(pid, instance_token, fields, true)
                    .map_err(|error| format!("{error:?}"));
                selected.and_then(|selected| {
                    os.apply_process_fields_if_instance(
                        pid,
                        instance_token,
                        mask,
                        priority,
                        selected,
                    )
                    .map(|_| ())
                })
            }
            #[cfg(not(test))]
            {
                Err("Process authority unavailable".into())
            }
        };
        if result.is_err() {
            return false;
        }
        instance_tokens.insert(pid, instance_token);
        true
    });

    no_identity_package_pids.retain(|&pid| {
        let Ok(instance_token) = os.get_process_instance_token(pid) else {
            return false;
        };
        if package_tokens.get(&pid) != Some(&instance_token)
            || seed_tokens
                .get(&pid)
                .is_some_and(|token| *token != instance_token)
            || (acknowledged && !os.current_logon_instance(pid, instance_token))
        {
            return false;
        }
        instance_tokens.insert(pid, instance_token);
        true
    });

    seed_pids.retain(|pid| {
        seed_tokens
            .get(pid)
            .is_some_and(|token| os.get_process_instance_token(*pid).ok() == Some(*token))
    });
    let retained: HashSet<_> = seed_pids.iter().copied().collect();
    seed_tokens.retain(|pid, _| retained.contains(pid));
    for &pid in seed_pids.iter() {
        if !saw_identity_seed
            && os
                .get_process_app_user_model_id(pid)?
                .is_some_and(|aumid| aumid.eq_ignore_ascii_case(expected_aumid))
        {
            saw_identity_seed = true;
        }
    }

    let new_managed_pids_added = managed_pids
        .iter()
        .filter(|pid| !before.contains(pid))
        .count();

    Ok(PostLaunchCorrectionOutcome {
        seed_pids: seed_pids.clone(),
        managed_pids,
        no_identity_package_pids,
        new_managed_pids_added,
        saw_identity_seed,
        instance_tokens,
    })
}

fn extend_with_descendants(snapshot: &LaunchProcessSnapshot, tracked_pids: &mut Vec<u32>) {
    let mut visited: HashSet<u32> = tracked_pids.iter().copied().collect();
    let mut stack = tracked_pids.clone();

    while let Some(parent_pid) = stack.pop() {
        if let Some(children) = snapshot.children_of.get(&parent_pid) {
            for &child_pid in children {
                if visited.insert(child_pid) {
                    tracked_pids.push(child_pid);
                    stack.push(child_pid);
                }
            }
        }
    }
}

fn installed_launch_pid_auto_managed<O: LaunchOs>(os: &O, pid: u32) -> bool {
    os.snapshot_process_tree()
        .map(|snapshot| is_auto_managed_installed_pid(&snapshot, pid))
        .unwrap_or(true)
}

fn is_auto_managed_installed_pid(snapshot: &LaunchProcessSnapshot, pid: u32) -> bool {
    match snapshot.names.get(&pid) {
        Some(name) => !is_excluded_installed_auto_process(name),
        None => true,
    }
}

fn retain_auto_managed_installed_pids(
    snapshot: &LaunchProcessSnapshot,
    tracked_pids: &mut Vec<u32>,
) {
    tracked_pids.retain(|&pid| is_auto_managed_installed_pid(snapshot, pid));
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PackageLocalPidCandidates {
    same_aumid_pids: Vec<u32>,
    no_identity_pids: Vec<u32>,
    instance_tokens: HashMap<u32, ProcessInstanceToken>,
}

fn collect_package_local_pids_from_live_snapshot<O: LaunchOs>(
    os: &O,
    install_root: &Path,
) -> Result<HashSet<u32>, String> {
    let snapshot = os.snapshot_process_tree()?;
    let mut pids = HashSet::new();

    for &pid in snapshot.names.keys() {
        if let Ok(image_path) = os.get_process_image_path(pid) {
            if path_is_under_root_case_insensitive(&image_path, install_root) {
                pids.insert(pid);
            }
        }
    }

    Ok(pids)
}

fn collect_package_local_pid_candidates_from_snapshot<O: LaunchOs>(
    os: &O,
    snapshot: &LaunchProcessSnapshot,
    install_root: &Path,
    expected_aumid: &str,
    prelaunch_package_pids: &HashSet<u32>,
    tracked_before: &HashSet<u32>,
) -> Result<PackageLocalPidCandidates, String> {
    let mut candidates = PackageLocalPidCandidates::default();

    for &pid in snapshot.names.keys() {
        if tracked_before.contains(&pid) || prelaunch_package_pids.contains(&pid) {
            continue;
        }

        let Ok(token) = os.get_process_instance_token(pid) else {
            continue;
        };
        let Ok(image_path) = os.get_process_image_path(pid) else {
            continue;
        };
        if !path_is_under_root_case_insensitive(&image_path, install_root) {
            continue;
        }

        let aumid = os.get_process_app_user_model_id(pid)?;
        if os.get_process_instance_token(pid).ok() != Some(token) {
            continue;
        }
        match aumid {
            Some(aumid) if !aumid.trim().is_empty() => {
                if aumid.eq_ignore_ascii_case(expected_aumid) {
                    candidates.same_aumid_pids.push(pid);
                    candidates.instance_tokens.insert(pid, token);
                }
            }
            _ => {
                candidates.no_identity_pids.push(pid);
                candidates.instance_tokens.insert(pid, token);
            }
        }
    }

    Ok(candidates)
}

fn normalize_path_for_prefix(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn path_is_under_root_case_insensitive(path: &Path, install_root: &Path) -> bool {
    let path_normalized = normalize_path_for_prefix(path);
    let root_normalized = normalize_path_for_prefix(install_root);

    path_normalized == root_normalized || path_normalized.starts_with(&(root_normalized + "\\"))
}

#[cfg(test)]
mod tests {
    include!("launch_known_app_tests.rs");
    use super::{
        collect_autorun_items, post_launch_correction_poll_with_os, record_started_pid,
        run_app_row_action_with_os, run_app_with_affinity_sync_with_os, run_launch_decision,
        AppRowAction, AppRowActionRequest, LaunchOs, LaunchProcessSnapshot,
        PostLaunchCorrectionRequest,
    };
    use crate::app::features::execution::RuntimeRegistry;
    use crate::app::models::{
        AppStateStorage, AppStatus, AppToRun, CoreGroup, CpuSchema, LogManager,
        ProcessInstanceToken,
    };
    use crate::app::shared::ids::{GroupId, RuleId};
    use os_api::{InstalledPackageRuntimeInfo, PriorityClass, ProcessSettingsApplyOutcome};
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    struct FakeLaunchOs {
        steam_calls: RefCell<Vec<u32>>,
        ordinary_calls: RefCell<Vec<(PathBuf, Vec<String>)>>,
        client_activation_calls: RefCell<Vec<String>>,
        launch_field_calls: RefCell<Vec<os_api::gaming::ProcessSettingsFields>>,
        affinity_calls: RefCell<Vec<(u32, usize)>>,
        affinity_results: HashMap<u32, Result<(), String>>,
        priority_calls: RefCell<Vec<(u32, PriorityClass)>>,
        priority_results: HashMap<u32, Result<(), String>>,
        instance_tokens: HashMap<u32, ProcessInstanceToken>,
        token_after_aumid: HashMap<u32, ProcessInstanceToken>,
        observed_token_changes: RefCell<HashMap<u32, ProcessInstanceToken>>,
        before_image_read: RefCell<Option<Box<dyn FnOnce()>>>,
        unavailable_tokens: HashSet<u32>,
        foreign_logon_pids: HashSet<u32>,
        focus_calls: RefCell<Vec<u32>>,
        focus_results: HashMap<u32, bool>,
        run_calls: RefCell<Vec<(PathBuf, Vec<String>, Vec<usize>, PriorityClass)>>,
        run_result: RefCell<Result<u32, String>>,
        activate_calls: RefCell<Vec<String>>,
        correction_calls: RefCell<Vec<PostLaunchCorrectionRequest>>,
        activate_result: RefCell<Result<u32, String>>,
        snapshot_result: RefCell<Result<LaunchProcessSnapshot, String>>,
        image_paths: HashMap<u32, PathBuf>,
        process_aumids: HashMap<u32, String>,
        installed_package_info: RefCell<Result<InstalledPackageRuntimeInfo, String>>,
    }

    impl Default for FakeLaunchOs {
        fn default() -> Self {
            Self {
                steam_calls: Default::default(),
                ordinary_calls: Default::default(),
                client_activation_calls: Default::default(),
                launch_field_calls: RefCell::new(Vec::new()),
                affinity_calls: RefCell::new(Vec::new()),
                affinity_results: HashMap::new(),
                priority_calls: RefCell::new(Vec::new()),
                priority_results: HashMap::new(),
                instance_tokens: HashMap::new(),
                token_after_aumid: HashMap::new(),
                observed_token_changes: RefCell::new(HashMap::new()),
                before_image_read: RefCell::new(None),
                unavailable_tokens: HashSet::new(),
                foreign_logon_pids: HashSet::new(),
                focus_calls: RefCell::new(Vec::new()),
                focus_results: HashMap::new(),
                run_calls: RefCell::new(Vec::new()),
                run_result: RefCell::new(Ok(0)),
                activate_calls: RefCell::new(Vec::new()),
                correction_calls: RefCell::new(Vec::new()),
                activate_result: RefCell::new(Ok(0)),
                snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot::default())),
                image_paths: HashMap::new(),
                process_aumids: HashMap::new(),
                installed_package_info: RefCell::new(Err("metadata unavailable".to_string())),
            }
        }
    }

    impl LaunchOs for FakeLaunchOs {
        fn activate_known_client(&self, app: &AppToRun) -> Result<(), String> {
            self.client_activation_calls
                .borrow_mut()
                .push(app.launch_target_label());
            Ok(())
        }

        fn launch_ordinary(&self, path: &std::path::Path, args: &[String]) -> Result<(), String> {
            self.ordinary_calls
                .borrow_mut()
                .push((path.into(), args.into()));
            Ok(())
        }

        fn launch_steam(&self, app_id: u32) -> Result<(), String> {
            self.steam_calls.borrow_mut().push(app_id);
            Ok(())
        }
        fn get_process_instance_token(&self, pid: u32) -> Result<ProcessInstanceToken, String> {
            if self.unavailable_tokens.contains(&pid) {
                return Err("process token temporarily unavailable".into());
            }
            if let Some(token) = self.observed_token_changes.borrow().get(&pid) {
                return Ok(*token);
            }
            Ok(self
                .instance_tokens
                .get(&pid)
                .copied()
                .unwrap_or(pid as u64 + 1_000))
        }

        fn current_logon_instance(&self, pid: u32, token: ProcessInstanceToken) -> bool {
            !self.foreign_logon_pids.contains(&pid)
                && self.get_process_instance_token(pid) == Ok(token)
        }

        fn apply_process_fields_if_instance(
            &self,
            pid: u32,
            expected_instance_token: ProcessInstanceToken,
            mask: usize,
            priority: PriorityClass,
            fields: os_api::gaming::ProcessSettingsFields,
        ) -> Result<ProcessSettingsApplyOutcome, String> {
            if self.get_process_instance_token(pid)? != expected_instance_token {
                return Err("process instance no longer matches the tracked PID".to_string());
            }
            if fields.affinity {
                self.affinity_calls.borrow_mut().push((pid, mask));
                self.affinity_results.get(&pid).cloned().unwrap_or(Ok(()))?;
            }
            if fields.priority {
                self.priority_calls.borrow_mut().push((pid, priority));
                self.priority_results.get(&pid).cloned().unwrap_or(Ok(()))?;
            }
            Ok(ProcessSettingsApplyOutcome {
                previous_affinity: mask,
                previous_priority: priority,
                affinity_changed: false,
                priority_changed: false,
            })
        }

        fn focus_window_by_pid(&self, pid: u32) -> bool {
            self.focus_calls.borrow_mut().push(pid);
            self.focus_results.get(&pid).copied().unwrap_or(false)
        }

        fn run_with_fields(
            &self,
            bin_path: PathBuf,
            args: Vec<String>,
            cores: &[usize],
            priority: PriorityClass,
            fields: os_api::gaming::ProcessSettingsFields,
        ) -> Result<os_api::gaming::ProcessLaunch, String> {
            self.launch_field_calls.borrow_mut().push(fields);
            self.run_calls
                .borrow_mut()
                .push((bin_path, args, cores.to_vec(), priority));
            self.run_result.borrow().clone().and_then(|pid| {
                self.get_process_instance_token(pid).map(|creation_token| {
                    os_api::gaming::ProcessLaunch {
                        pid,
                        creation_token,
                    }
                })
            })
        }

        fn activate_application(&self, aumid: &str) -> Result<u32, String> {
            self.activate_calls.borrow_mut().push(aumid.to_string());
            self.activate_result.borrow().clone()
        }

        fn start_post_launch_correction(&self, request: PostLaunchCorrectionRequest) {
            self.correction_calls.borrow_mut().push(request);
        }

        fn snapshot_process_tree(&self) -> Result<LaunchProcessSnapshot, String> {
            self.snapshot_result.borrow().clone()
        }

        fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String> {
            if let Some(change) = self.before_image_read.borrow_mut().take() {
                change();
            }
            self.image_paths
                .get(&pid)
                .cloned()
                .ok_or_else(|| format!("missing image path for pid {pid}"))
        }

        fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String> {
            if let Some(&token) = self.token_after_aumid.get(&pid) {
                self.observed_token_changes.borrow_mut().insert(pid, token);
            }
            Ok(self.process_aumids.get(&pid).cloned())
        }

        fn resolve_installed_package_runtime_info(
            &self,
            _aumid: &str,
        ) -> Result<InstalledPackageRuntimeInfo, String> {
            self.installed_package_info.borrow().clone()
        }
    }

    fn steam_rule(app_id: u32) -> AppToRun {
        let mut app = AppToRun::new_path(
            PathBuf::from(r"C:\game.url"),
            vec![format!("steam://rungameid/{app_id}")],
            PathBuf::from(r"C:\Steam\steam.exe"),
            PriorityClass::Normal,
            false,
        );
        app.launch_target = crate::app::models::LaunchTarget::Steam {
            app_id,
            executable: None,
        };
        app.args.clear();
        app
    }

    fn s112_installed_request() -> PostLaunchCorrectionRequest {
        let mut state = crate::app::features::execution::control::tests::fixture();
        state.process_monitoring_enabled = false;
        state.groups.truncate(1);
        state.groups[0].programs[0] = AppToRun::new_installed(
            "Example".into(),
            "Example!Main".into(),
            PriorityClass::Normal,
            false,
        );
        let rules = crate::app::features::rules::RulesContext::from_storage(&state);
        state.rule_identities = Some(rules.to_persisted_identities());
        let runtime = RuntimeRegistry::new();
        runtime.control_handle().configure(&state);
        let group = rules.group_id_for_index(0).unwrap();
        let rule = rules.rule_id_for_index(0, 0).unwrap();
        let app = state.groups[0].programs[0].clone();
        let key = rules.runtime_key(&app, &rule);
        runtime.add_running_app_with_token(&key, 42, 1042, group.clone(), rule.clone());
        PostLaunchCorrectionRequest {
            source: app,
            wake: None,
            running_apps: runtime.running_apps_handle(),
            installed_package_tracking: runtime.installed_package_tracking_handle(),
            app_key: key,
            initial_pid: Some(42),
            initial_token: Some(1042),
            group_id: group,
            rule_id: rule,
            group_cores: vec![0],
            priority: PriorityClass::Normal,
            expected_aumid: "Example!Main".into(),
            installed_package_info: Some(InstalledPackageRuntimeInfo {
                aumid: "Example!Main".into(),
                package_family_name: "Example".into(),
                install_root: r"C:\Package".into(),
            }),
            prelaunch_package_pids: HashSet::new(),
            manage_descendants: false,
            fields: os_api::gaming::ProcessSettingsFields::default(),
            scheduling: runtime.scheduling_handle(),
            control: runtime.control_handle(),
        }
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn s116_acknowledged_installed_activation_schedules_exact_main_with_monitor_off() {
        let request = s112_installed_request();
        request.running_apps.blocking_write().apps.clear();
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                names: HashMap::from([
                    (42, "Main.exe".into()),
                    (43, "Foreign.exe".into()),
                    (44, "Other.exe".into()),
                    (45, "backgroundtaskhost.exe".into()),
                ]),
                ..Default::default()
            })),
            process_aumids: HashMap::from([
                (42, "Example!Main".into()),
                (43, "Example!Main".into()),
                (44, "Other!Main".into()),
                (45, "Example!Main".into()),
            ]),
            image_paths: HashMap::from([(42, r"C:\Package\Main.exe".into())]),
            foreign_logon_pids: HashSet::from([43]),
            ..Default::default()
        };
        let mut activated = false;
        super::activate_installed_with_os(
            super::InstalledActivationRequest {
                source: super::correction_source(&request).unwrap(),
                wake: None,
                running_apps: request.running_apps.clone(),
                installed_package_tracking: request.installed_package_tracking.clone(),
                scheduling: request.scheduling.clone(),
                control: request.control.clone(),
            },
            |aumid| {
                assert_eq!(aumid, "Example!Main");
                activated = true;
                Ok(())
            },
            &os,
        )
        .unwrap();
        assert!(activated);
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(request.running_apps.blocking_read().apps.is_empty());
        let pending =
            os.correction_calls.borrow_mut().pop().expect(
                "ordinary activation acknowledgement must enter bounded initial correction",
            );
        assert_eq!(
            pending.initial_pid, None,
            "acknowledgement must not invent a PID"
        );
        assert_eq!(pending.initial_token, None);
        let mut seeds: Vec<_> = pending.initial_pid.into_iter().collect();
        let outcome = super::post_launch_correction_poll_with_fields(
            &os,
            &pending.expected_aumid,
            &mut seeds,
            &mut HashMap::new(),
            &pending.group_cores,
            pending.priority,
            pending.installed_package_info.as_ref(),
            &pending.prelaunch_package_pids,
            pending.manage_descendants,
            pending.fields,
            &pending.scheduling,
            Some((
                &pending.control,
                super::correction_source(&pending).unwrap(),
            )),
            pending.initial_pid.is_none(),
        )
        .unwrap();
        super::complete_post_launch_correction(&pending, &outcome, &os).unwrap();
        assert_eq!(outcome.managed_pids, [42]);
        assert!(os.affinity_calls.borrow().contains(&(42, 1)));
        assert_eq!(os.affinity_calls.borrow().len(), 1);
        assert_eq!(
            pending.running_apps.blocking_read().apps[&pending.app_key]
                .pid_instance_tokens
                .get(&42),
            Some(&1042)
        );
    }
    fn s112_late_helper_outcome() -> super::PostLaunchCorrectionOutcome {
        super::PostLaunchCorrectionOutcome {
            seed_pids: vec![42],
            managed_pids: vec![42],
            no_identity_package_pids: vec![43],
            new_managed_pids_added: 0,
            saw_identity_seed: true,
            instance_tokens: HashMap::from([(42, 1042), (43, 1043)]),
        }
    }

    #[test]
    fn s116_installed_correction_rechecks_source_at_each_native_write() {
        for late_helper in [false, true] {
            let mut request = s112_installed_request();
            request.source.manage_descendants = true;
            request.manage_descendants = true;
            let mut changed = crate::app::features::execution::control::tests::fixture();
            changed.groups.truncate(1);
            changed.groups[0].programs[0] = request.source.clone();
            changed.rule_identities = Some(crate::app::features::rules::PersistedRuleIdentities {
                groups: vec![crate::app::features::rules::PersistedGroupIdentity {
                    id: request.group_id.clone(),
                    rule_ids: vec![request.rule_id.clone()],
                }],
                next_group_id: 20,
                next_rule_id: 20,
            });
            let control = request.control.clone();
            control.configure(&changed);
            changed.groups[0].programs[0].manage_descendants = false;
            let os = FakeLaunchOs {
                before_image_read: RefCell::new(Some(Box::new(move || {
                    control.configure(&changed);
                }))),
                snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                    names: HashMap::from([(42, "Main.exe".into()), (43, "Helper.exe".into())]),
                    children_of: HashMap::from([(42, vec![43])]),
                })),
                image_paths: HashMap::from([
                    (42, r"C:\Package\Main.exe".into()),
                    (43, r"C:\Package\Helper.exe".into()),
                ]),
                process_aumids: HashMap::from([(42, "Example!Main".into())]),
                ..Default::default()
            };
            if late_helper {
                super::complete_post_launch_correction(&request, &s112_late_helper_outcome(), &os)
                    .unwrap();
            } else {
                request
                    .control
                    .with_source(super::correction_source(&request).unwrap(), |_| Ok(()))
                    .unwrap();
                super::post_launch_correction_poll_with_fields(
                    &os,
                    &request.expected_aumid,
                    &mut vec![42],
                    &mut HashMap::from([(42, 1042)]),
                    &request.group_cores,
                    request.priority,
                    None,
                    &HashSet::new(),
                    request.manage_descendants,
                    request.fields,
                    &request.scheduling,
                    Some((
                        &request.control,
                        super::correction_source(&request).unwrap(),
                    )),
                    false,
                )
                .unwrap();
            }
            assert!(
                os.affinity_calls.borrow().is_empty(),
                "stale source applied affinity (late helper: {late_helper})"
            );
            assert!(
                os.priority_calls.borrow().is_empty(),
                "stale source applied priority (late helper: {late_helper})"
            );
        }
    }

    #[test]
    fn s116_installed_correction_rejects_reused_package_candidates() {
        for has_aumid in [false, true] {
            let request = s112_installed_request();
            let os = FakeLaunchOs {
                snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                    names: HashMap::from([(42, "Main.exe".into()), (43, "Helper.exe".into())]),
                    ..Default::default()
                })),
                image_paths: HashMap::from([
                    (42, r"C:\Package\Main.exe".into()),
                    (43, r"C:\Package\Helper.exe".into()),
                ]),
                process_aumids: if has_aumid {
                    HashMap::from([(42, "Example!Main".into()), (43, "Example!Main".into())])
                } else {
                    HashMap::from([(42, "Example!Main".into())])
                },
                token_after_aumid: HashMap::from([(43, 2043)]),
                ..Default::default()
            };
            let outcome = super::post_launch_correction_poll_with_fields(
                &os,
                &request.expected_aumid,
                &mut vec![42],
                &mut HashMap::from([(42, 1042)]),
                &request.group_cores,
                request.priority,
                request.installed_package_info.as_ref(),
                &HashSet::new(),
                request.manage_descendants,
                request.fields,
                &request.scheduling,
                Some((
                    &request.control,
                    super::correction_source(&request).unwrap(),
                )),
                false,
            )
            .unwrap();
            super::complete_post_launch_correction(&request, &outcome, &os).unwrap();
            assert!(
                !os.affinity_calls.borrow().iter().any(|(pid, _)| *pid == 43),
                "package evidence was rebound to reused PID (AUMID: {has_aumid})"
            );
            assert!(!request.running_apps.blocking_read().apps[&request.app_key]
                .pid_instance_tokens
                .contains_key(&43));
        }

        // A helper accepted by one GUI poll must retain that exact lifetime in
        // the next poll, rather than adopting a replacement with the same PID.
        let request = s112_installed_request();
        let mut os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                names: HashMap::from([(42, "Main.exe".into()), (43, "Helper.exe".into())]),
                ..Default::default()
            })),
            image_paths: HashMap::from([
                (42, r"C:\Package\Main.exe".into()),
                (43, r"C:\Package\Helper.exe".into()),
            ]),
            process_aumids: HashMap::from([(42, "Example!Main".into())]),
            ..Default::default()
        };
        let mut seeds = vec![42];
        let mut seed_tokens = HashMap::from([(42, 1042)]);
        let first = super::post_launch_correction_poll_with_fields(
            &os,
            &request.expected_aumid,
            &mut seeds,
            &mut seed_tokens,
            &request.group_cores,
            request.priority,
            request.installed_package_info.as_ref(),
            &HashSet::new(),
            request.manage_descendants,
            request.fields,
            &request.scheduling,
            Some((
                &request.control,
                super::correction_source(&request).unwrap(),
            )),
            false,
        )
        .unwrap();
        let publication = super::complete_post_launch_correction(&request, &first, &os).unwrap();
        assert_eq!(publication.helpers, [43]);
        seeds = first.seed_pids;
        for &pid in &publication.helpers {
            seed_tokens.insert(pid, first.instance_tokens[&pid]);
        }
        seeds.extend(publication.helpers);
        os.observed_token_changes.borrow_mut().insert(43, 2043);
        os.image_paths.insert(43, r"C:\Other\Unrelated.exe".into());
        os.affinity_calls.borrow_mut().clear();
        os.priority_calls.borrow_mut().clear();
        let second = super::post_launch_correction_poll_with_fields(
            &os,
            &request.expected_aumid,
            &mut seeds,
            &mut seed_tokens,
            &request.group_cores,
            request.priority,
            request.installed_package_info.as_ref(),
            &HashSet::new(),
            request.manage_descendants,
            request.fields,
            &request.scheduling,
            Some((
                &request.control,
                super::correction_source(&request).unwrap(),
            )),
            false,
        )
        .unwrap();
        super::complete_post_launch_correction(&request, &second, &os).unwrap();
        assert!(
            !os.affinity_calls.borrow().iter().any(|(pid, _)| *pid == 43),
            "retained helper PID was rebound on the next poll"
        );
        assert!(!second.seed_pids.contains(&43));
        assert_eq!(
            request.running_apps.blocking_read().apps[&request.app_key]
                .pid_instance_tokens
                .get(&43),
            Some(&1043)
        );
    }
    #[test]
    fn s112_last_stable_poll_applies_new_package_helper_immediately() {
        let request = s112_installed_request();
        let os = FakeLaunchOs::default();
        let published =
            super::complete_post_launch_correction(&request, &s112_late_helper_outcome(), &os)
                .unwrap();
        assert_eq!(published.helpers, [43]);
        assert!(
            os.affinity_calls.borrow().contains(&(43, 1)),
            "a last-poll helper must receive a guarded initial correction with monitor Off"
        );
    }
    #[test]
    fn s112_stale_installed_callback_cannot_revert_current_group_membership() {
        let request = s112_installed_request();
        {
            let mut apps = request.running_apps.try_write().unwrap();
            apps.apps.get_mut(&request.app_key).unwrap().group_id = GroupId("new-group".into());
        }
        // The saved authority has also changed, as it does after the writer moves the rule.
        let mut state = crate::app::features::execution::control::tests::fixture();
        state.groups.truncate(1);
        state.groups[0].programs[0] = request.source.clone();
        state.groups[0].cores = vec![1];
        state.rule_identities = Some(crate::app::features::rules::PersistedRuleIdentities {
            groups: vec![crate::app::features::rules::PersistedGroupIdentity {
                id: GroupId("new-group".into()),
                rule_ids: vec![request.rule_id.clone()],
            }],
            next_group_id: 20,
            next_rule_id: 20,
        });
        request.control.configure(&state);
        let _ = super::complete_post_launch_correction(
            &request,
            &s112_late_helper_outcome(),
            &FakeLaunchOs::default(),
        );
        let apps = request.running_apps.try_read().unwrap();
        assert_eq!(
            apps.apps[&request.app_key].group_id,
            GroupId("new-group".into())
        );
    }
    #[test]
    fn s103_focus_refuses_helper_when_main_identity_is_missing_in_all_routes() {
        let app = steam_rule(987654);
        let runtime = RuntimeRegistry::new();
        let mut log = LogManager::default();
        assert!(runtime.add_running_app_with_token(
            &app.get_key(),
            10,
            1010,
            group_id(0),
            rule_id(0)
        ));
        let os = FakeLaunchOs {
            focus_results: HashMap::from([(10, true)]),
            ..Default::default()
        };
        assert!(matches!(
            super::focus_existing_app(&runtime, &mut log, app.clone(), &os),
            super::LaunchDispatchOutcome::Rejected(_)
        ));
        run_launch_decision(
            &runtime,
            &mut log,
            group_id(0),
            rule_id(0),
            app,
            vec![0, 1],
            &os,
        );
        assert!(os.focus_calls.borrow().is_empty());
    }
    #[test]
    fn steam_dispatch_does_not_register_or_schedule_the_launcher_and_coalesces_retries() {
        let app = steam_rule(1867241);
        super::finish_steam_launch(&rule_id(0), 1867241);
        let runtime = RuntimeRegistry::new();
        let mut log = LogManager::default();
        let os = FakeLaunchOs::default();
        for _ in 0..2 {
            assert_eq!(
                run_launch_decision(
                    &runtime,
                    &mut log,
                    group_id(0),
                    rule_id(0),
                    app.clone(),
                    vec![0, 1],
                    &os
                ),
                super::LaunchDispatchOutcome::Accepted
            );
        }
        assert_eq!(*os.steam_calls.borrow(), vec![1867241]);
        assert!(os.run_calls.borrow().is_empty());
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(runtime
            .running_apps_handle()
            .blocking_read()
            .apps
            .is_empty());
        super::finish_steam_launch(&rule_id(0), 1867241);
    }
    #[test]
    fn steam_focus_uses_selected_game_not_an_extra_process() {
        let mut app = steam_rule(1867242);
        app.launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 1867242,
            executable: Some(PathBuf::from(r"C:\Game\game.exe")),
        };
        let runtime = RuntimeRegistry::new();
        assert!(runtime.add_running_app_with_token(
            &app.get_key(),
            10,
            1010,
            group_id(0),
            rule_id(0)
        ));
        assert!(runtime.add_running_app_with_token(
            &app.get_key(),
            20,
            1020,
            group_id(0),
            rule_id(0)
        ));
        runtime
            .running_apps_handle()
            .blocking_write()
            .apps
            .get_mut(&app.get_key())
            .unwrap()
            .primary_instances
            .insert(20, 1020);
        let os = FakeLaunchOs {
            image_paths: HashMap::from([
                (10, PathBuf::from(r"C:\Game\helper.exe")),
                (20, PathBuf::from(r"C:\Game\game.exe")),
            ]),
            focus_results: HashMap::from([(10, true), (20, true)]),
            ..Default::default()
        };
        let mut log = LogManager::default();
        assert_eq!(
            super::focus_existing_app(&runtime, &mut log, app, &os),
            super::LaunchDispatchOutcome::Accepted
        );
        assert_eq!(*os.focus_calls.borrow(), vec![20]);
        assert!(os.steam_calls.borrow().is_empty());
    }

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
            state.version = 5;
            state.groups = vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![0, 1],
                programs: vec![
                    sample_app(),
                    AppToRun::new_path(
                        PathBuf::from(r"C:\two.lnk"),
                        vec!["--autorun".to_string()],
                        PathBuf::from(r"C:\two.exe"),
                        PriorityClass::High,
                        true,
                    ),
                ],
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
        })))
    }

    fn enable_scheduling(
        state: &mut AppStateStorage,
        group_index: usize,
        rule_index: usize,
        features: &[&str],
    ) {
        use crate::app::features::{
            rules::RulesContext,
            system_tweaks::{
                config::{Association, ModulePreference, Profile, TuningConfig, TuningEnvelope},
                platform::contract::{ActivationEligibility, ActivationMode, ProfileId},
                tweaks,
            },
        };
        let rules = RulesContext::from_storage(state);
        state.rule_identities = Some(rules.to_persisted_identities());
        let profile = ProfileId::new(1).unwrap();
        let modules = tweaks::modules()
            .into_iter()
            .filter(|module| features.contains(&module.descriptor().id.as_str()))
            .map(|module| ModulePreference {
                feature: module.descriptor().id,
                enabled: true,
                mode: ActivationMode::ForegroundSession,
                config: module.editor().default_config(),
            })
            .collect();
        state.tuning = TuningEnvelope::from_config(&TuningConfig {
            profiles: vec![Profile {
                version: 1,
                id: profile,
                name: "Game".into(),
                modules,
            }],
            associations: vec![Association {
                version: 1,
                group: rules.group_id_for_index(group_index).unwrap(),
                rule: rules.rule_id_for_index(group_index, rule_index).unwrap(),
                profile,
                eligibility: ActivationEligibility::Foreground,
            }],
            next_profile: 2,
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn handoff_fresh_launch_reserves_fields_only_on_enabled_platform() {
        let state = sample_state();
        enable_scheduling(
            &mut state.write().unwrap(),
            0,
            0,
            &["cpu_sets", "process_priority"],
        );
        let runtime = RuntimeRegistry::new();
        let mut log = LogManager::default();
        let app = state.read().unwrap().groups[0].programs[0].clone();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(321)),
            ..Default::default()
        };
        assert_eq!(
            run_app_with_affinity_sync_with_os(&state, &runtime, &mut log, 0, 0, app, &os),
            super::LaunchDispatchOutcome::Accepted
        );
        assert_eq!(
            os.launch_field_calls.borrow().as_slice(),
            &[os_api::gaming::ProcessSettingsFields {
                affinity: !crate::app::features::system_tweaks::MODULES_ENABLED,
                priority: !crate::app::features::system_tweaks::MODULES_ENABLED
            }]
        );
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(os.priority_calls.borrow().is_empty());
    }

    #[test]
    fn handoff_manual_fix_reserves_priority_only_on_enabled_platform() {
        let state = sample_state();
        enable_scheduling(&mut state.write().unwrap(), 0, 0, &["process_priority"]);
        let mut runtime = RuntimeRegistry::new();
        let mut log = LogManager::default();
        let app = state.read().unwrap().groups[0].programs[0].clone();
        let expected_priority = app.priority;
        runtime.add_running_app_with_token(&app.get_key(), 322, 1322, group_id(0), rule_id(0));
        let os = FakeLaunchOs::default();
        assert_eq!(
            run_app_row_action_with_os(
                &state,
                &mut runtime,
                &mut log,
                row_request(app, AppRowAction::Fix),
                &os
            ),
            super::LaunchDispatchOutcome::Accepted
        );
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(322, 3)]);
        if crate::app::features::system_tweaks::MODULES_ENABLED {
            assert!(
                os.priority_calls.borrow().is_empty(),
                "classic Fix must not race the module's baseline capture"
            );
        } else {
            assert_eq!(
                os.priority_calls.borrow().as_slice(),
                &[(322, expected_priority)]
            );
        }
    }

    fn sample_app() -> AppToRun {
        AppToRun::new_path(
            PathBuf::from(r"C:\game.lnk"),
            vec!["--fullscreen".to_string()],
            PathBuf::from(r"C:\game.exe"),
            PriorityClass::High,
            false,
        )
    }

    fn group_id(value: usize) -> GroupId {
        GroupId(format!("group-{value}"))
    }

    fn rule_id(value: usize) -> RuleId {
        RuleId(format!("rule-{value}"))
    }

    fn set_settings_matched(runtime: &RuntimeRegistry, app: &AppToRun, matched: bool) {
        let running_apps = runtime.running_apps_handle();
        running_apps
            .try_write()
            .unwrap()
            .apps
            .get_mut(&app.get_key())
            .unwrap()
            .settings_matched = matched;
    }

    fn row_request(app: AppToRun, action: AppRowAction) -> AppRowActionRequest {
        AppRowActionRequest {
            group_index: 0,
            program_index: 0,
            app,
            action,
        }
    }

    fn assert_no_row_action_os_calls(os: &FakeLaunchOs) {
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(os.priority_calls.borrow().is_empty());
        assert!(os.focus_calls.borrow().is_empty());
        assert!(os.run_calls.borrow().is_empty());
        assert!(os.activate_calls.borrow().is_empty());
    }

    #[test]
    fn test_collect_autorun_items_preserves_indices() {
        let items = collect_autorun_items(&sample_state());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 0);
        assert_eq!(items[0].1, 1);
        assert_eq!(
            items[0].2.bin_path(),
            Some(PathBuf::from(r"C:\two.exe").as_path())
        );
        assert_eq!(items[0].2.args, vec!["--autorun".to_string()]);
    }

    #[test]
    fn test_already_running_with_focus_reapplies_settings_without_launch() {
        let runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 41, 1041, group_id(3), rule_id(4)));
        assert!(runtime.add_pid_to_existing_app_with_token(&app_key, 42, 1042));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            focus_results: HashMap::from([(41, false), (42, true)]),
            run_result: RefCell::new(Ok(999)),
            ..Default::default()
        };

        run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(3),
            rule_id(4),
            app,
            vec![0, 2],
            &os,
        );

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(41, 5), (42, 5)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(41, PriorityClass::High), (42, PriorityClass::High)]
        );
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("window focused")));
    }

    #[test]
    fn test_already_running_without_focus_does_not_launch_duplicate() {
        let runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 77, 1077, group_id(0), rule_id(0)));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(555)),
            ..Default::default()
        };

        run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![1, 3],
            &os,
        );

        assert!(os.run_calls.borrow().is_empty());
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("no window found to focus")));
    }

    #[test]
    fn test_already_running_apply_failure_is_rejected_mismatched_and_never_logs_reapplied() {
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 77, 1077, group_id(0), rule_id(0)));
        assert!(runtime.add_pid_to_existing_app_with_token(&app_key, 78, 1078));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            affinity_results: HashMap::from([(
                77,
                Err("SetProcessAffinityMask: Windows protects this process; affinity and priority changes are not permitted".to_string()),
            )]),
            focus_results: HashMap::from([(77, true)]),
            ..Default::default()
        };

        let outcome = run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![0, 1],
            &os,
        );

        assert!(
            matches!(outcome, super::LaunchDispatchOutcome::Rejected(message)
            if message.contains("PID 77")
                && message.contains("Windows protects this process"))
        );
        assert_eq!(
            runtime.get_app_status_sync(&app_key),
            AppStatus::SettingsMismatch
        );
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(77, 3), (78, 3)]);
        assert!(os.focus_calls.borrow().is_empty());
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("PID 77")));
        assert!(!log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("settings reapplied")));
    }

    #[test]
    fn test_row_fix_reapplies_without_focusing_and_marks_confirmed_match() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 41, 1041, group_id(0), rule_id(0)));
        assert!(runtime.add_pid_to_existing_app_with_token(&app_key, 42, 1042));
        set_settings_matched(&runtime, &app, false);
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            focus_results: HashMap::from([(41, true)]),
            ..Default::default()
        };

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Fix),
            &os,
        );

        assert_eq!(outcome, super::LaunchDispatchOutcome::Accepted);
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(41, 3), (42, 3)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(41, PriorityClass::High), (42, PriorityClass::High)]
        );
        assert!(os.focus_calls.borrow().is_empty());
        assert!(os.run_calls.borrow().is_empty());
        assert!(os.activate_calls.borrow().is_empty());
        assert_eq!(runtime.get_app_status_sync(&app_key), AppStatus::Running);
    }

    #[test]
    fn test_row_fix_setter_failure_stays_mismatched_and_is_rejected() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 77, 1077, group_id(0), rule_id(0)));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            affinity_results: HashMap::from([(77, Err("access denied".to_string()))]),
            focus_results: HashMap::from([(77, true)]),
            ..Default::default()
        };

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Fix),
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert!(os.focus_calls.borrow().is_empty());
        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(
            runtime.get_app_status_sync(&app_key),
            AppStatus::SettingsMismatch
        );
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("access denied")));
    }

    #[test]
    fn test_row_fix_busy_fails_closed_without_os_calls() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        assert!(runtime.add_running_app_with_token(
            &app.get_key(),
            77,
            1077,
            group_id(0),
            rule_id(0)
        ));
        let running_apps = runtime.running_apps_handle();
        let _write_guard = running_apps.try_write().unwrap();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Fix),
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert_no_row_action_os_calls(&os);
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("temporarily busy")));
    }

    #[test]
    fn test_row_fix_disappeared_entry_fails_closed_without_os_calls() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 77, 1077, group_id(0), rule_id(0)));
        runtime
            .running_apps_handle()
            .try_write()
            .unwrap()
            .remove_app(&app_key);
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Fix),
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert_no_row_action_os_calls(&os);
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("not running")));
    }

    #[test]
    fn test_row_focus_only_focuses_and_does_not_reapply_or_launch() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 91, 1091, group_id(0), rule_id(0)));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            focus_results: HashMap::from([(91, true)]),
            ..Default::default()
        };

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Focus),
            &os,
        );

        assert_eq!(outcome, super::LaunchDispatchOutcome::Accepted);
        assert_eq!(os.focus_calls.borrow().as_slice(), &[91]);
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(os.priority_calls.borrow().is_empty());
        assert!(os.run_calls.borrow().is_empty());
        assert!(os.activate_calls.borrow().is_empty());
    }

    #[test]
    fn test_row_focus_rejects_reused_pid_without_focusing() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 91, 500, group_id(0), rule_id(0)));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            instance_tokens: HashMap::from([(91, 501)]),
            focus_results: HashMap::from([(91, true)]),
            ..Default::default()
        };

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Focus),
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert!(os.focus_calls.borrow().is_empty());
    }

    #[test]
    fn test_row_focus_busy_fails_closed_without_os_calls() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        assert!(runtime.add_running_app_with_token(
            &app.get_key(),
            91,
            1091,
            group_id(0),
            rule_id(0)
        ));
        let running_apps = runtime.running_apps_handle();
        let _write_guard = running_apps.try_write().unwrap();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Focus),
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert_no_row_action_os_calls(&os);
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("temporarily busy")));
    }

    #[test]
    fn test_row_focus_disappeared_entry_fails_closed_without_os_calls() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 91, 1091, group_id(0), rule_id(0)));
        runtime
            .running_apps_handle()
            .try_write()
            .unwrap()
            .remove_app(&app_key);
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Focus),
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert_no_row_action_os_calls(&os);
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("not running")));
    }

    #[test]
    fn test_row_run_launches_not_running_app() {
        let state = sample_state();
        let mut runtime = RuntimeRegistry::new();
        let app = sample_app();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(5150)),
            ..Default::default()
        };

        let outcome = run_app_row_action_with_os(
            &state,
            &mut runtime,
            &mut log_manager,
            row_request(app, AppRowAction::Run),
            &os,
        );

        assert_eq!(outcome, super::LaunchDispatchOutcome::Accepted);
        assert_eq!(os.run_calls.borrow().len(), 1);
        assert!(os.focus_calls.borrow().is_empty());
    }

    #[test]
    fn test_running_registry_contention_rejects_path_launch_without_duplicate() {
        let runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 77, 1077, group_id(0), rule_id(0)));
        let running_apps = runtime.running_apps_handle();
        let _write_guard = running_apps.try_write().unwrap();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(555)),
            ..Default::default()
        };

        let outcome = run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![1, 3],
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert!(os.run_calls.borrow().is_empty());
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("temporarily busy")));
    }

    #[test]
    fn test_running_registry_contention_rejects_installed_activation_without_duplicate() {
        let runtime = RuntimeRegistry::new();
        let app = AppToRun::new_installed(
            "Sample Store App".to_string(),
            "Sample.Package!App".to_string(),
            PriorityClass::Normal,
            false,
        );
        let app_key = app.get_key();
        assert!(runtime.add_running_app_with_token(&app_key, 88, 1088, group_id(0), rule_id(0)));
        let running_apps = runtime.running_apps_handle();
        let _write_guard = running_apps.try_write().unwrap();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Err("activation should not be attempted".to_string())),
            ..Default::default()
        };

        let outcome = run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![0],
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert!(os.activate_calls.borrow().is_empty());
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("temporarily busy")));
    }

    #[test]
    fn test_invalid_affinity_core_rejects_launch_without_os_calls() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        let outcome = run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            sample_app(),
            vec![usize::BITS as usize],
            &os,
        );

        assert!(matches!(outcome, super::LaunchDispatchOutcome::Rejected(_)));
        assert_no_row_action_os_calls(&os);
    }

    #[test]
    fn test_missing_group_logs_critical_and_stops() {
        let state = sample_state();
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        run_app_with_affinity_sync_with_os(
            &state,
            &runtime,
            &mut log_manager,
            99,
            0,
            sample_app(),
            &os,
        );

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(
            log_manager
                .entries
                .iter()
                .filter(|entry| entry.message.contains("Group index 99 not found"))
                .count(),
            2
        );
    }

    #[test]
    fn test_fresh_launch_success_tracks_pid_in_runtime_registry() {
        let state = sample_state();
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(4242)),
            ..Default::default()
        };
        let app = sample_app();
        let app_key = app.get_key();

        run_app_with_affinity_sync_with_os(&state, &runtime, &mut log_manager, 0, 0, app, &os);

        assert_eq!(runtime.get_running_app_pids(&app_key), Some(vec![4242]));
        assert_eq!(
            runtime
                .running_apps_handle()
                .try_read()
                .unwrap()
                .membership_generation(),
            Some(1)
        );
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message == "App started with PID: 4242"));
    }

    #[test]
    fn test_record_started_pid_appends_to_existing_runtime_entry_without_duplicates() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let key = sample_app().get_key();
        assert!(runtime.add_running_app_with_token(&key, 41, 1041, group_id(1), rule_id(2)));

        record_started_pid(
            &runtime,
            &mut log_manager,
            &key,
            5150,
            6150,
            group_id(1),
            rule_id(2),
        );
        record_started_pid(
            &runtime,
            &mut log_manager,
            &key,
            5150,
            6150,
            group_id(1),
            rule_id(2),
        );

        let pids = runtime.get_running_app_pids(&key).unwrap();
        assert!(pids.contains(&41));
        assert!(pids.contains(&5150));
        assert_eq!(pids.iter().filter(|&&pid| pid == 5150).count(), 1);
        assert_eq!(
            runtime
                .running_apps_handle()
                .try_read()
                .unwrap()
                .membership_generation(),
            Some(2)
        );
        assert!(log_manager.entries.iter().any(|entry| entry
            .message
            .contains("New instance of existing app started")));
    }

    #[test]
    fn test_launch_failure_logs_important_and_sticky_error() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Err("boom".to_string())),
            ..Default::default()
        };

        run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            sample_app(),
            vec![0],
            &os,
        );
        assert_eq!(
            runtime
                .running_apps_handle()
                .try_read()
                .unwrap()
                .membership_generation(),
            Some(0)
        );

        assert_eq!(
            log_manager
                .entries
                .iter()
                .filter(|entry| entry.message == "ERROR: boom")
                .count(),
            2
        );
    }

    #[test]
    fn test_installed_activation_without_initial_token_waits_for_rediscovery() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let app = AppToRun::new_installed(
            "Spotify".into(),
            "Spotify!OwnedApp".into(),
            PriorityClass::Normal,
            false,
        );
        let key = app.get_key();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Ok(4321)),
            unavailable_tokens: HashSet::from([4321]),
            ..Default::default()
        };
        let outcome = run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![0, 2],
            &os,
        );
        assert_eq!(outcome, super::LaunchDispatchOutcome::Accepted);
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(os.priority_calls.borrow().is_empty());
        assert!(runtime.get_running_app_pids(&key).is_none());
        assert_eq!(os.correction_calls.borrow().len(), 1);
        assert_eq!(os.correction_calls.borrow()[0].initial_pid, Some(4321));
        assert_eq!(os.correction_calls.borrow()[0].initial_token, None);
    }

    #[test]
    fn test_installed_launch_uses_activation_and_immediate_apply() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let app = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );
        let app_key = app.get_key();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Ok(4321)),
            ..Default::default()
        };

        run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![0, 2],
            &os,
        );

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(
            os.activate_calls.borrow().as_slice(),
            &["SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string()]
        );
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(4321, 5)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(4321, PriorityClass::High)]
        );
        assert_eq!(runtime.get_running_app_pids(&app_key), Some(vec![4321]));
    }

    #[test]
    fn test_installed_launch_skips_background_host_activation_pid() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let app = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );
        let app_key = app.get_key();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Ok(4321)),
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(4321, "backgroundTaskHost.exe".to_string())]),
            })),
            ..Default::default()
        };

        run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![0, 2],
            &os,
        );

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(
            os.activate_calls.borrow().as_slice(),
            &["SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string()]
        );
        assert!(os.affinity_calls.borrow().is_empty());
        assert!(os.priority_calls.borrow().is_empty());
        assert_eq!(runtime.get_running_app_pids(&app_key), None);
    }

    #[test]
    fn test_post_launch_correction_poll_attaches_descendants_and_reapplies_settings() {
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::from([(50, vec![51]), (51, vec![52])]),
                names: HashMap::from([
                    (50, "Spotify.exe".to_string()),
                    (51, "SpotifyHelper.exe".to_string()),
                    (52, "SpotifyHelper.exe".to_string()),
                ]),
            })),
            process_aumids: HashMap::from([(
                50,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };
        let mut tracked_pids = vec![50];

        let outcome = post_launch_correction_poll_with_os(
            &os,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &mut tracked_pids,
            &[1, 3],
            PriorityClass::AboveNormal,
            None,
            &std::collections::HashSet::new(),
            true,
        )
        .unwrap();

        assert_eq!(outcome.seed_pids, vec![50, 51, 52]);
        assert_eq!(outcome.managed_pids, vec![50, 51, 52]);
        assert!(outcome.no_identity_package_pids.is_empty());
        assert_eq!(outcome.new_managed_pids_added, 2);
        assert!(outcome.saw_identity_seed);
        assert_eq!(
            os.affinity_calls.borrow().as_slice(),
            &[(50, 10), (51, 10), (52, 10)]
        );
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[
                (50, PriorityClass::AboveNormal),
                (51, PriorityClass::AboveNormal),
                (52, PriorityClass::AboveNormal),
            ]
        );
    }

    #[test]
    fn test_post_launch_correction_does_not_walk_descendants_when_disabled() {
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::from([(50, vec![51])]),
                names: HashMap::from([
                    (50, "Spotify.exe".to_string()),
                    (51, "SpotifyHelper.exe".to_string()),
                ]),
            })),
            process_aumids: HashMap::from([(
                50,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };
        let mut seed_pids = vec![50];

        let outcome = post_launch_correction_poll_with_os(
            &os,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &mut seed_pids,
            &[1, 3],
            PriorityClass::AboveNormal,
            None,
            &HashSet::new(),
            false,
        )
        .unwrap();

        assert_eq!(outcome.seed_pids, vec![50]);
        assert_eq!(outcome.managed_pids, vec![50]);
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(50, 10)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(50, PriorityClass::AboveNormal)]
        );
    }

    #[test]
    fn test_post_launch_correction_uses_background_host_as_seed_only() {
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::from([(60, vec![61])]),
                names: HashMap::from([
                    (60, "backgroundTaskHost.exe".to_string()),
                    (61, "Spotify.exe".to_string()),
                ]),
            })),
            process_aumids: HashMap::from([
                (
                    60,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    61,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
            ]),
            ..Default::default()
        };
        let mut seed_pids = vec![60];

        let outcome = post_launch_correction_poll_with_os(
            &os,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &mut seed_pids,
            &[1, 3],
            PriorityClass::AboveNormal,
            None,
            &std::collections::HashSet::new(),
            true,
        )
        .unwrap();

        assert_eq!(outcome.seed_pids, vec![60, 61]);
        assert_eq!(outcome.managed_pids, vec![61]);
        assert!(outcome.no_identity_package_pids.is_empty());
        assert_eq!(outcome.new_managed_pids_added, 1);
        assert!(outcome.saw_identity_seed);
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(61, 10)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(61, PriorityClass::AboveNormal)]
        );
    }

    #[test]
    fn test_post_launch_correction_poll_attaches_only_new_package_local_pids() {
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (40, "Spotify.exe".to_string()),
                    (41, "SpotifyLauncher.exe".to_string()),
                    (42, "SpotifyHelper.exe".to_string()),
                    (43, "SpotifyHelper.exe".to_string()),
                    (44, "SpotifyHelper.exe".to_string()),
                ]),
            })),
            image_paths: HashMap::from([
                (
                    40,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\Spotify.exe"),
                ),
                (
                    41,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyLauncher.exe"),
                ),
                (
                    42,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyHelper.exe"),
                ),
                (43, PathBuf::from(r"C:\Other\SpotifyHelper.exe")),
                (
                    44,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\Widget.exe"),
                ),
            ]),
            process_aumids: HashMap::from([
                (
                    40,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    42,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    44,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Widget".to_string(),
                ),
            ]),
            ..Default::default()
        };
        let mut tracked_pids = vec![40];
        let package_info = InstalledPackageRuntimeInfo {
            aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
            install_root: PathBuf::from(r"C:\Program Files\WindowsApps\Spotify"),
        };
        let prelaunch = std::collections::HashSet::from([41]);

        let outcome = post_launch_correction_poll_with_os(
            &os,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &mut tracked_pids,
            &[0, 1],
            PriorityClass::High,
            Some(&package_info),
            &prelaunch,
            true,
        )
        .unwrap();

        assert_eq!(outcome.seed_pids, vec![40, 42]);
        assert_eq!(outcome.managed_pids, vec![40, 42]);
        assert_eq!(outcome.no_identity_package_pids, Vec::<u32>::new());
        assert_eq!(outcome.new_managed_pids_added, 1);
        assert!(outcome.saw_identity_seed);
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(40, 3), (42, 3)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(40, PriorityClass::High), (42, PriorityClass::High)]
        );
    }

    #[test]
    fn test_installed_launch_with_metadata_resolve_failure_keeps_soft_fallback() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let app = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );
        let app_key = app.get_key();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Ok(4321)),
            installed_package_info: RefCell::new(Err("metadata unavailable".into())),
            ..Default::default()
        };

        run_launch_decision(
            &runtime,
            &mut log_manager,
            group_id(0),
            rule_id(0),
            app,
            vec![0, 2],
            &os,
        );

        assert_eq!(runtime.get_running_app_pids(&app_key), Some(vec![4321]));
        assert_eq!(
            os.activate_calls.borrow().as_slice(),
            &["SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string()]
        );
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(4321, 5)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(4321, PriorityClass::High)]
        );
        assert!(!log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("metadata unavailable")));
    }
}
