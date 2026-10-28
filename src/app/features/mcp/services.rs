use super::{
    operations::{ApiError, ApiResult},
    policy::AgentAccess,
    server::LiveAccess,
};
use crate::app::{
    features::{
        rules::commands::{BaseCommand, PreferencePatch, RuleInput},
        system_tweaks::{RequestFence, SaveError, TuningClient, TuningRequest},
    },
    shared::ids::{GroupId, RuleId},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct NativeSlots(Arc<tokio::sync::Semaphore>);
impl Default for NativeSlots {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::Semaphore::new(4)))
    }
}

#[derive(Clone)]
pub(super) struct Services {
    pub tuning: TuningClient,
    pub wake: eframe::egui::Context,
    pub cpu: crate::app::features::cpu_load::CpuReadHandle,
    pub runs: Arc<crate::app::features::execution::managed_runs::ManagedRuns>,
    pub running: Arc<tokio::sync::RwLock<crate::app::models::RunningApps>>,
    pub scheduling: Arc<crate::app::features::system_tweaks::tweaks::scheduling::SchedulingHandoff>,
    pub control: Arc<crate::app::features::execution::control::Authority>,
    pub registry: crate::app::features::system_tweaks::platform::registry::Registry,
    pub api_epoch: String,
    pub storage_profile_id: String,
    native_slots: NativeSlots,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Page {
    offset: usize,
    limit: Option<usize>,
    group_id: Option<String>,
    revision: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigArgs {
    #[serde(default)]
    image: Option<String>,
    expected_revision: String,
    request_key: String,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    rule_id: Option<String>,
    #[serde(default)]
    target_group_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cpus: Option<Vec<usize>>,
    #[serde(default)]
    run_all: bool,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    rule: Option<RuleInput>,
    #[serde(default)]
    preferences: Option<PreferencePatch>,
}
fn required<T>(value: Option<T>, name: &str) -> Result<T, ApiError> {
    value.ok_or_else(|| ApiError::invalid(format!("{name} is required")))
}
pub(super) const REPLACEMENT_RULE_FIELDS: &[&str] = &[
    "name",
    "args",
    "tracked_process_names",
    "manage_descendants",
    "autorun",
    "priority",
];
pub(super) fn validate_rule_replacement(arguments: &Value) -> Result<(), ApiError> {
    let rule = arguments
        .get("rule")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ApiError::invalid("update_rule requires a complete rule object; read list_rules first")
        })?;
    if ["path", "aumid", "steam_app_id"]
        .iter()
        .filter(|key| rule.contains_key(**key))
        .count()
        != 1
    {
        return Err(ApiError::invalid("update_rule requires a complete replacement with exactly one path or aumid; preserve the current target from list_rules"));
    }
    if rule.contains_key("steam_app_id") && !rule.contains_key("steam_executable") {
        return Err(ApiError::invalid("A Steam replacement must preserve steam_executable (path or explicit null for automatic detection). Read list_rules first."));
    }
    for field in REPLACEMENT_RULE_FIELDS {
        if !rule.contains_key(*field) {
            return Err(ApiError::invalid(format!("update_rule requires a complete replacement: missing rule.{field}. Read list_rules and preserve every other editable value; omitted fields are not a patch.")));
        }
    }
    Ok(())
}
pub(super) fn decode<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, ApiError> {
    serde_json::from_value(arguments).map_err(|e| ApiError::invalid(e.to_string()))
}
pub(super) fn bounded_row<T: Serialize>(row: &T) -> Result<Value, ApiError> {
    struct Buffer(Vec<u8>);
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len() + bytes.len() > 48 * 1024 {
                return Err(std::io::ErrorKind::FileTooLarge.into());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer(&mut buffer, row).map_err(|_| {
        ApiError::new(
            "row_too_large",
            "A retained legacy row exceeds the 48 KiB API row limit",
        )
    })?;
    serde_json::from_slice(&buffer.0)
        .map_err(|_| ApiError::new("serialization", "Invalid configuration row"))
}
#[derive(Serialize)]
struct RuleRow<'a> {
    group_id: &'a GroupId,
    rule_id: Option<RuleId>,
    app: &'a crate::app::models::AppToRun,
}
#[derive(Serialize)]
struct GroupRow<'a> {
    group_id: Option<GroupId>,
    name: &'a str,
    cpus: &'a [usize],
    hidden: bool,
    run_all: bool,
    rule_count: usize,
}
impl Services {
    pub(super) async fn native_query(
        &self,
        operation: impl FnOnce() -> ApiResult + Send + 'static,
    ) -> ApiResult {
        let permit = self
            .native_slots
            .0
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|_| ApiError::new("worker", "Native operation worker failed"))?
    }
    pub(super) fn from_app(
        app: &crate::app::runtime::AppState,
        wake: eframe::egui::Context,
        api_epoch: String,
        native_slots: NativeSlots,
    ) -> Option<Self> {
        Some(Self {
            tuning: app.tuning_client()?,
            wake,
            cpu: app.cpu_load.read_handle(),
            runs: app.runtime.managed_runs.clone(),
            running: app.runtime.running_apps_handle(),
            scheduling: app.runtime.scheduling_handle(),
            control: app.runtime.control_handle(),
            registry: app.tuning_editors().clone(),
            api_epoch,
            storage_profile_id: os_api::agent::profile_key(
                &crate::app::models::AppStateStorage::active_data_dir(),
            )
            .ok()?,
            native_slots,
        })
    }
    pub fn read(&self, name: &str, arguments: Value) -> Option<ApiResult> {
        if name == "read_system_tweaks" {
            return Some(Ok(self.read_tweaks()));
        }
        if matches!(
            name,
            "read_cpu_current"
                | "read_run_cpu"
                | "read_cpu_history"
                | "read_cpu_changes"
                | "read_cpu_contributors"
        ) {
            return Some(self.read_cpu(name, arguments));
        }
        if !matches!(
            name,
            "get_app_state"
                | "get_topology"
                | "list_groups"
                | "list_rules"
                | "read_process_control"
        ) {
            return None;
        }
        Some((|| {
            let snapshot = self.tuning.snapshot();
            if name == "get_app_state" {
                let state = &snapshot.state;
                return Ok(
                    json!({"configuration_revision":snapshot.revision.to_string(),"save_status":format!("{:?}",snapshot.operation),"preferences":{"theme_index":state.theme_index,"process_monitoring_enabled":state.process_monitoring_enabled,"cpu_load_monitoring_enabled":state.cpu_load_monitoring_enabled,"cpu_load_history_seconds":state.cpu_load_history_seconds,"start_minimized":state.start_minimized,"windows_event_log_diagnostics_enabled":state.windows_event_log_diagnostics_enabled},"group_count":state.groups.len()}),
                );
            }
            let page: Page = decode(arguments)?;
            if page.offset > 100000 || page.limit.is_some_and(|limit| limit == 0 || limit > 256) {
                return Err(ApiError::invalid("Invalid page bounds"));
            }
            if page
                .revision
                .as_deref()
                .is_some_and(|revision| revision != snapshot.revision.to_string())
            {
                return Err(ApiError::new(
                    "revision_conflict",
                    "Configuration changed; restart pagination",
                ));
            }
            let limit = page.limit.unwrap_or(50);
            if name == "read_process_control" {
                let view = self.control.view();
                let mut rows = Vec::new();
                let mut bytes = 0;
                for notice in view.notices.iter().skip(page.offset).take(limit) {
                    let row = json!({"image":notice.image,"name":notice.name,"owner_rule_id":notice.owner,"different_settings":notice.different,"claimants":notice.claimants.iter().map(|id|json!({"rule_id":id,"group_id":view.rules.iter().find(|rule|&rule.rule==id).map(|rule|&rule.group),"label":view.label(id)})).collect::<Vec<_>>()});
                    let size = row.to_string().len();
                    if bytes + size > 48 * 1024 {
                        break;
                    }
                    bytes += size;
                    rows.push(row);
                }
                let next = page.offset + rows.len();
                return Ok(
                    json!({"configuration_revision":snapshot.revision.to_string(),"rows":rows,"total":view.notices.len(),"next_offset":(next<view.notices.len()).then_some(next),"error":view.error}),
                );
            }
            if name == "get_topology" {
                let schema = &snapshot.state.cpu_schema;
                let total = schema
                    .clusters
                    .iter()
                    .map(|cluster| cluster.cores.len())
                    .sum::<usize>();
                let rows:Vec<_>=schema.clusters.iter().flat_map(|cluster|cluster.cores.iter().map(|core|json!({"cpu":core.index,"cluster":cluster.name,"type":core.core_type,"label":core.label}))).skip(page.offset).take(limit).collect();
                let next = page.offset + rows.len();
                return Ok(
                    json!({"model":schema.model,"logical_cpu_count":crate::app::models::effective_total_threads(),"rows":rows,"total":total,"next_offset":(next<total).then_some(next)}),
                );
            }
            let mut rows = Vec::new();
            let mut bytes = 0;
            let total;
            if name == "list_groups" {
                total = snapshot.state.groups.len();
                for (index, group) in snapshot
                    .state
                    .groups
                    .iter()
                    .enumerate()
                    .skip(page.offset)
                    .take(limit)
                {
                    let row = bounded_row(&GroupRow {
                        group_id: snapshot.rules.group_id_for_index(index),
                        name: &group.name,
                        cpus: &group.cores,
                        hidden: group.is_hidden,
                        run_all: group.run_all_button,
                        rule_count: group.programs.len(),
                    })?;
                    bytes += row.to_string().len();
                    if bytes > 48 * 1024 {
                        break;
                    }
                    rows.push(row);
                }
            } else {
                let group_id = GroupId(required(page.group_id, "group_id")?);
                let group_index = snapshot
                    .rules
                    .group_index_for_id(&group_id)
                    .ok_or_else(|| ApiError::new("missing_group", "Unknown GroupId"))?;
                let group = &snapshot.state.groups[group_index];
                total = group.programs.len();
                for (index, app) in group
                    .programs
                    .iter()
                    .enumerate()
                    .skip(page.offset)
                    .take(limit)
                {
                    let row = bounded_row(&RuleRow {
                        group_id: &group_id,
                        rule_id: snapshot.rules.rule_id_for_index(group_index, index),
                        app,
                    })?;
                    bytes += row.to_string().len();
                    if bytes > 48 * 1024 {
                        break;
                    }
                    rows.push(row);
                }
            }
            let next = page.offset + rows.len();
            Ok(
                json!({"configuration_revision":snapshot.revision.to_string(),"total":total,"next_offset":(next<total).then_some(next),"rows":rows}),
            )
        })())
    }
    pub async fn configure(
        &self,
        name: &str,
        arguments: Value,
        agent: &AgentAccess,
        live: &Arc<LiveAccess>,
        generation: u64,
    ) -> ApiResult {
        if name == "update_rule" {
            validate_rule_replacement(&arguments)?;
        }
        let args: ConfigArgs = decode(arguments)?;
        let _key = args.request_key;
        let revision = args
            .expected_revision
            .parse::<u64>()
            .map_err(|_| ApiError::invalid("expected_revision must be a decimal string"))?;
        let snapshot = self.tuning.snapshot();
        if snapshot.revision != revision {
            return Err(ApiError::new(
                "revision_conflict",
                format!(
                    "Expected {revision}; current durable revision is {}",
                    snapshot.revision
                ),
            ));
        }
        let group = args.group_id.map(GroupId);
        let rule = args.rule_id.map(RuleId);
        let mut created = json!({});
        let request = if name == "choose_process_owner" {
            TuningRequest::ChooseProcessOwner(
                crate::app::features::execution::control::policy::Choice {
                    image: required(args.image, "image")?,
                    owner: required(rule.clone(), "rule_id")?,
                },
            )
        } else {
            let command = match name {
                "create_group" => {
                    created["group_id"] = json!(format!(
                        "group-{}",
                        snapshot.rules.to_persisted_identities().next_group_id
                    ));
                    BaseCommand::CreateGroup {
                        name: required(args.name, "name")?,
                        cpus: required(args.cpus, "cpus")?,
                        run_all: args.run_all,
                    }
                }
                "update_group" => BaseCommand::UpdateGroup {
                    group: required(group, "group_id")?,
                    name: required(args.name, "name")?,
                    cpus: required(args.cpus, "cpus")?,
                    run_all: args.run_all,
                    hidden: args.hidden,
                },
                "delete_group" => BaseCommand::RemoveGroup {
                    group: required(group, "group_id")?,
                },
                "move_group" => BaseCommand::MoveGroup {
                    group: required(group, "group_id")?,
                    index: required(args.index, "index")?,
                },
                "create_rule" | "update_rule" => {
                    let group = required(group, "group_id")?;
                    let app = required(args.rule, "rule")?
                        .into_app()
                        .map_err(ApiError::invalid)?;
                    if name == "create_rule" {
                        created["rule_id"] = json!(format!(
                            "rule-{}",
                            snapshot.rules.to_persisted_identities().next_rule_id
                        ));
                        BaseCommand::CreateRule {
                            group,
                            app: Box::new(app),
                        }
                    } else {
                        BaseCommand::UpdateRule {
                            group,
                            rule: required(rule, "rule_id")?,
                            app: Box::new(app),
                        }
                    }
                }
                "delete_rule" => BaseCommand::RemoveRule {
                    group: required(group, "group_id")?,
                    rule: required(rule, "rule_id")?,
                },
                "move_rule" => BaseCommand::MoveRule {
                    group: required(group, "group_id")?,
                    rule: required(rule, "rule_id")?,
                    target: GroupId(required(args.target_group_id, "target_group_id")?),
                    index: required(args.index, "index")?,
                },
                "update_preferences" => {
                    BaseCommand::Preferences(required(args.preferences, "preferences")?)
                }
                _ => {
                    return Err(ApiError::new(
                        "unknown_tool",
                        "Unsupported configuration command",
                    ))
                }
            };
            TuningRequest::ConfigureBase(command)
        };
        if !live.current(agent, generation) {
            return Err(ApiError::new(
                "revoked",
                "Access was revoked before admission",
            ));
        }
        let id = self
            .tuning
            .request_guarded(
                request,
                Some(revision),
                Some(RequestFence {
                    generation: live.generation.clone(),
                    expected: generation,
                }),
            )
            .map_err(save_error)?;
        self.wake.request_repaint();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(receipt) = self.tuning.receipt(id) {
                receipt.result.map_err(save_error)?;
                self.wake.request_repaint();
                created["configuration_revision"] = json!(receipt.revision.to_string());
                created["persistence"] = json!("durable");
                return Ok(created);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ApiError::new("indeterminate", "Configuration operation is still unresolved; inspect current revision before retrying"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
pub(super) fn save_error(error: SaveError) -> ApiError {
    match error {
        SaveError::Pending => ApiError::busy(),
        SaveError::Invalid => ApiError::new("revision_or_validation_conflict", "The writer rejected the stale, revoked or invalid configuration; saved state was preserved"),
        SaveError::Storage => ApiError::new("storage", "The state was not saved; the previous configuration was retained"),
        SaveError::Recovering => ApiError::new("not_ready", "The application worker is recovering or shutting down"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{models::AppStateStorage, runtime::AppState};
    use std::sync::{Condvar, Mutex, RwLock};
    use std::time::Duration;

    #[test]
    fn native_budget_survives_listener_recreation_and_cancelled_callers() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let app = AppState::new_for_test(
                Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {
                    s.groups.clear()
                }))),
                4,
            );
            let mut mcp = super::super::runtime::McpRuntime::default();
            let first =
                Services::from_app(&app, Default::default(), "epoch".into(), mcp.native_slots())
                    .unwrap();
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            struct Release(Arc<(Mutex<bool>, Condvar)>);
            impl Drop for Release {
                fn drop(&mut self) {
                    *self.0 .0.lock().unwrap() = true;
                    self.0 .1.notify_all();
                }
            }
            let release = Release(gate.clone());
            let (started, received) = std::sync::mpsc::channel();
            let mut callers = Vec::new();
            for _ in 0..4 {
                let service = first.clone();
                let gate = gate.clone();
                let started = started.clone();
                callers.push(tokio::spawn(async move {
                    service
                        .native_query(move || {
                            let (lock, wake) = &*gate;
                            let open = lock.lock().unwrap();
                            started.send(()).unwrap();
                            let _guard = wake.wait_while(open, |open| !*open).unwrap();
                            Ok(json!({}))
                        })
                        .await
                }));
            }
            for _ in 0..4 {
                received.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            for caller in callers {
                caller.abort();
            }
            let restarted =
                Services::from_app(&app, Default::default(), "epoch".into(), mcp.native_slots())
                    .unwrap();
            let result = restarted.native_query(|| Ok(json!({}))).await;
            assert!(Arc::ptr_eq(
                &first.native_slots.0,
                &restarted.native_slots.0
            ));
            assert_eq!(
                result.unwrap_err().code,
                "busy",
                "Restarting the listener must not admit a fifth native worker"
            );
            drop(release);
            tokio::time::timeout(Duration::from_secs(3), async {
                while restarted.native_slots.0.available_permits() != 4 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(restarted.native_query(|| Ok(json!({}))).await.is_ok());
        });
    }
}

#[cfg(test)]
mod steam_rule_contract_tests {
    #[test]
    fn replacement_preserves_the_game_selection_including_explicit_automatic() {
        let mut value = serde_json::json!({"rule":{"name":"Game","steam_app_id":570,"args":[],"tracked_process_names":[],"manage_descendants":false,"autorun":false,"priority":"Normal"}});
        assert!(super::validate_rule_replacement(&value).is_err());
        value["rule"]["steam_executable"] = serde_json::Value::Null;
        assert!(super::validate_rule_replacement(&value).is_ok());
        value["rule"]["path"] = serde_json::json!("C:\\Steam\\steam.exe");
        assert!(super::validate_rule_replacement(&value).is_err());
    }
}

#[cfg(test)]
mod process_control_tests {
    use super::*;
    use crate::app::{
        features::execution::control::{
            policy::Choice,
            tests::{fixture, observe},
        },
        models::AppStateStorage,
        runtime::AppState,
    };
    use std::sync::RwLock;
    use std::time::Duration;
    #[test]
    fn s110_mcp_reads_conflicts_and_uses_the_durable_owner_command() {
        let state = fixture();
        let app = AppState::new_for_test(Arc::new(RwLock::new(state.clone())), 4);
        let services =
            Services::from_app(&app, Default::default(), "test".into(), Default::default())
                .unwrap();
        observe(
            &app.runtime.control_handle(),
            &state,
            &[0, 1],
            1,
            r"C:\Shared\helper.exe",
        );
        let response = services
            .read("read_process_control", json!({}))
            .unwrap()
            .unwrap();
        assert_eq!(response["rows"][0]["owner_rule_id"], Value::Null);
        assert_eq!(
            response["rows"][0]["claimants"].as_array().unwrap().len(),
            2
        );
        let owner = app.runtime.control_handle().view().rules[1].rule.clone();
        let request = services
            .tuning
            .request_guarded(
                TuningRequest::ChooseProcessOwner(Choice {
                    image: response["rows"][0]["image"].as_str().unwrap().into(),
                    owner: owner.clone(),
                }),
                Some(services.tuning.snapshot().revision),
                None,
            )
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            if let Some(receipt) = services.tuning.receipt(request) {
                receipt.result.unwrap();
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        let saved = services.tuning.snapshot();
        assert_eq!(
            serde_json::to_value(&saved.state.groups).unwrap(),
            serde_json::to_value(&state.groups).unwrap()
        );
        assert_eq!(
            crate::app::features::execution::control::policy::Policy::read(&saved.state)
                .unwrap()
                .choices[0]
                .owner,
            owner
        );
        let _: AppStateStorage = state;
    }
}
