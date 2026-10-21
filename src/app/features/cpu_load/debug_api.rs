use super::{model::GraphTarget, RuleTarget};
use crate::app::features::diagnostics::debug_mcp::{self, Command};
use crate::app::runtime::AppState;
use crate::app::shared::ids::{GroupId, RuleId};
use eframe::egui;
use serde_json::{json, Value};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_graph_read_does_not_expand_retained_history() {
        use super::super::{
            contributors::ContributorView,
            history::{HistoricalSample, Overview, Series, SeriesPoint},
            runtime::GraphData,
        };
        let mut series = Series::default();
        series.points = (0..1801)
            .map(|i| SeriesPoint {
                seconds: i as f64,
                timestamp: std::time::UNIX_EPOCH,
                value: None,
                partial: false,
                parked: None,
            })
            .collect();
        let sample = HistoricalSample::default();
        let overview = Overview::default();
        let contributors = ContributorView::default();
        let data = GraphData {
            series: &series,
            overview: &overview,
            sample: &sample,
            hovered: None,
            contributors: &contributors,
            pending: true,
            available: true,
        };
        let compact = rendered_graph(&data, &[], Some(0), false);
        assert_eq!(compact["series_points"], 1801);
        assert!(compact.get("series").is_none());
        assert!(compact.to_string().len() < 1024);
        let detailed = rendered_graph(&data, &[], Some(0), true);
        assert_eq!(detailed["series"].as_array().unwrap().len(), 1801);
        assert!(detailed["series"][0]["load"].is_null());
        assert!(detailed["series"][0]["parked"].is_null());
    }
    #[test]
    fn current_pid_quality_preserves_missing_values_and_deduplicates_owners() {
        let one = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("r".into()),
        };
        let two = RuleTarget {
            rule: RuleId("s".into()),
            ..one.clone()
        };
        let mut list = super::super::process_list::ProcessList::from_tracking(
            &std::collections::HashMap::from([(one, vec![(42, 10)]), (two, vec![(42, 10)])]),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        list.record_cores(
            2,
            &[os_api::cpu_load::ProcessCpuSample {
                pid: 42,
                token: 10,
                cpus: vec![Some(4.0), None],
            }],
        );
        let value = current_quality(&list, None);
        assert_eq!(value["total"], 1);
        assert_eq!(value["incomplete"], 1);
        assert!(value["rows"][0]["whole_machine_cpu_percent"].is_null());
        assert_eq!(
            value["rows"][0]["reasons"],
            json!([
                "process_cpu_time_unavailable",
                "scheduler_mapping_incomplete",
                "process_name_unavailable"
            ])
        );
    }
}
pub(crate) fn wait_observation(
    app: &AppState,
    ui_settled: bool,
) -> Result<debug_mcp::WaitObservation, String> {
    let (completed_seconds, duration) = app.cpu_load.debug_wait_history()?;
    let saved = app
        .persistent_state
        .try_read()
        .map_err(|_| "Settings busy")?;
    let enabled = app
        .cpu_load
        .preference_override
        .unwrap_or(saved.cpu_load_monitoring_enabled);
    let history_seconds = app.cpu_load.history_seconds_override.unwrap_or_else(|| {
        crate::app::models::AppStateStorage::normalize_cpu_load_history_seconds(
            saved.cpu_load_history_seconds,
        )
    });
    let committed = app.cpu_load.preference_override.is_none()
        && app.cpu_load.history_seconds_override.is_none()
        && app.cpu_load.preference_error.is_none()
        && (!enabled || duration == history_seconds);
    Ok(debug_mcp::WaitObservation {
        completed_seconds,
        enabled,
        history_seconds,
        settings_committed: committed,
        collection_released: !enabled && app.cpu_load.debug_disabled_acknowledged() == Some(true),
        ui_settled,
    })
}
pub(crate) fn quality(
    app: &AppState,
    rendered: Option<&Value>,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    if limit == 0 || limit > 100 || offset > 100_000 {
        return Err("Use limit 1..100 and offset <=100000".into());
    }
    let target = match &app.cpu_load.graph {
        Some(GraphTarget::Process { target, .. }) => Some(target),
        _ => None,
    };
    let cpu = rendered
        .and_then(|v| v["cpu"].as_u64())
        .map(|v| v as usize)
        .or(app.cpu_load.selected_cpu)
        .unwrap_or(0);
    let seconds = rendered.and_then(|v| v["sample_seconds"].as_f64());
    let snapshot = app.cpu_load.snapshot();
    let history = app
        .cpu_load
        .debug_quality(target, seconds, cpu, offset, limit)?;
    Ok(
        json!({"scope":if target.is_some(){"app"}else if app.cpu_load.tracked_only(){"tracked_union"}else{"system"},"history":history,"current_error":snapshot.error,"current_detail_error":snapshot.detail_error,"current_error_category":crate::app::features::diagnostics::render_profile::error_code(snapshot.detail_error.as_deref().or(snapshot.error.as_deref())),"current_tracked_instances":current_quality(&snapshot.live_processes,target),"rendered_sample_seconds":seconds}),
    )
}
fn current_quality(list: &super::process_list::ProcessList, target: Option<&RuleTarget>) -> Value {
    let mut seen = std::collections::HashSet::new();
    let mut missing = Vec::new();
    let mut count = 0;
    for row in list
        .rows
        .iter()
        .filter(|r| target.is_none_or(|t| r.target.as_ref() == Some(t)))
    {
        if !seen.insert((row.pid, row.token)) {
            continue;
        }
        count += 1;
        let mut reasons = Vec::new();
        if !list.ready {
            reasons.push("tracking_snapshot_updating");
        }
        if row.total.is_none() {
            reasons.push("process_cpu_time_unavailable");
        }
        if row.known_cores == 0 {
            reasons.push("scheduler_mapping_unavailable");
        } else if row.known_cores < row.cpu_count {
            reasons.push("scheduler_mapping_incomplete");
        }
        if row.name() == "Name unavailable" {
            reasons.push("process_name_unavailable");
        }
        if !reasons.is_empty() {
            missing.push(json!({"pid":row.pid,"instance":row.token.to_string(),"name":row.name(),"known_cpus":row.known_cores,"cpu_count":row.cpu_count,"whole_machine_cpu_percent":row.total,"reasons":reasons}));
        }
    }
    json!({"ready":list.ready,"time_unix_ms":list.time.and_then(|t|t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok()).map(|d|d.as_millis()),"total":count,"incomplete":missing.len(),"truncated":missing.len()>100,"rows":missing.into_iter().take(100).collect::<Vec<_>>(),"note":"Current exact tracked instances only, deduplicated by PID/lifetime. Not a historical PID roster or full host-process coverage. Mapping reasons describe known availability, not an inferred native root cause."})
}

pub(crate) fn groups(app: &AppState) -> Result<Value, String> {
    let state = app
        .persistent_state
        .try_read()
        .map_err(|_| "Settings busy")?;
    Ok(json!(state.groups.iter().enumerate().map(|(g,group)|json!({"id":app.rules.group_id_for_index(g).map(|id|id.0),"name":group.name,"cpus":group.cores,"rules":group.programs.iter().enumerate().map(|(r,p)|json!({"id":app.rules.rule_id_for_index(g,r).map(|id|id.0),"name":p.name})).collect::<Vec<_>>()})).collect::<Vec<_>>()))
}
pub(crate) fn graph(app: &AppState, _ctx: &egui::Context) -> Value {
    let runtime = &app.cpu_load;
    let snapshot = runtime.snapshot();
    let target = match &runtime.graph {
        None => Value::Null,
        Some(GraphTarget::System) => json!({"kind":"system"}),
        Some(GraphTarget::Group { id, .. }) => json!({"kind":"group","group_id":id.0}),
        Some(GraphTarget::Process { target, .. }) => {
            json!({"kind":"app","group_id":target.group.0,"rule_id":target.rule.0})
        }
    };
    json!({"target":target,"selected_cpu":runtime.selected_cpu,"all_cpus":runtime.show_all_cpus,"tracked_only":runtime.tracked_only(),"view":if runtime.show_processes{"processes"}else{"graphs"},"pinned_seconds":runtime.inspection.pinned.map(|t|t.seconds),"instances":snapshot.instances,"observed_threads":snapshot.threads,"history_bytes":snapshot.history_bytes,"error":snapshot.error,"detail_error":snapshot.detail_error,"worker_stopped":runtime.is_stopped(),"disabled_acknowledged":runtime.debug_disabled_acknowledged()})
}
pub(crate) fn apply(app: &mut AppState, command: &Command) -> Result<(), String> {
    match command {
        Command::OpenGraph { group_id, rule_id } => {
            if !app.cpu_load_enabled() {
                return Err("Enable CPU monitoring first".into());
            }
            let id = GroupId(group_id.clone());
            let g = app
                .rules
                .group_index_for_id(&id)
                .ok_or("Unknown group ID")?;
            let state = app
                .persistent_state
                .try_read()
                .map_err(|_| "Settings busy")?;
            let group = state.groups.get(g).ok_or("Group removed")?;
            let target = if let Some(rule_id) = rule_id {
                let rule = RuleId(rule_id.clone());
                let r = app
                    .rules
                    .rule_index_for_id(g, &rule)
                    .ok_or("Unknown rule ID")?;
                GraphTarget::Process {
                    target: RuleTarget { group: id, rule },
                    name: group.programs.get(r).ok_or("Rule removed")?.name.clone(),
                }
            } else {
                GraphTarget::Group {
                    id,
                    name: group.name.clone(),
                    cpus: group.cores.clone(),
                }
            };
            drop(state);
            app.ui.app_settings_open = false;
            app.ui.cpu_monitor_open = false;
            app.cpu_load.open(target);
        }
        Command::CloseGraph {} => app.cpu_load.close(),
        Command::SelectCpu { cpu } => {
            let snapshot = app.cpu_load.snapshot();
            if app.cpu_load.graph.is_none() || *cpu >= snapshot.cpus.len() {
                return Err("CPU unavailable in current graph".into());
            }
            if !app.cpu_load.show_all_cpus {
                let id = match app.cpu_load.graph.as_ref().unwrap() {
                    GraphTarget::System => None,
                    GraphTarget::Group { id, .. } => Some(id),
                    GraphTarget::Process { target, .. } => Some(&target.group),
                };
                if let Some(id) = id {
                    let g = app.rules.group_index_for_id(id).ok_or("Group removed")?;
                    let saved = app
                        .persistent_state
                        .try_read()
                        .map_err(|_| "Settings busy")?;
                    if !saved.groups.get(g).is_some_and(|g| g.cores.contains(cpu)) {
                        return Err("CPU is filtered out; enable All CPUs first".into());
                    }
                }
            }
            app.cpu_load.selected_cpu = Some(*cpu);
        }
        Command::GraphOptions {
            all_cpus,
            tracked_only,
            view,
        } => {
            if app.cpu_load.graph.is_none() {
                return Err("Open a graph first".into());
            }
            if tracked_only.is_some()
                && matches!(app.cpu_load.graph, Some(GraphTarget::Process { .. }))
            {
                return Err("Tracked-only filter is group-only".into());
            }
            if view
                .as_ref()
                .is_some_and(|v| v != "graphs" && v != "processes")
            {
                return Err("View must be graphs or processes".into());
            }
            if let Some(v) = all_cpus {
                if !*v && matches!(app.cpu_load.graph, Some(GraphTarget::System)) {
                    return Err("System graphs always include every logical CPU".into());
                }
                app.cpu_load.show_all_cpus = *v;
            }
            if let Some(v) = tracked_only {
                app.cpu_load.set_tracked_only(*v);
            }
            if let Some(v) = view {
                app.cpu_load.show_processes = v == "processes";
            }
        }
        Command::Inspect { samples_back } => {
            let target = match &app.cpu_load.graph {
                Some(GraphTarget::Process { target, .. }) => Some(target.clone()),
                Some(_) => None,
                None => return Err("Open a graph first".into()),
            };
            let cpu = app.cpu_load.selected_cpu.ok_or("Select a CPU first")?;
            let series = app.cpu_load.series(target.as_ref(), cpu);
            let point = series
                .points
                .len()
                .checked_sub(samples_back.saturating_add(1))
                .and_then(|i| series.points.get(i))
                .copied()
                .ok_or("Sample outside retained history")?;
            app.cpu_load.inspection.pinned = Some(super::history::SampleTime {
                seconds: point.seconds,
                wall: point.timestamp,
            });
        }
        Command::Live {} => app.cpu_load.inspection = Default::default(),
        Command::Monitor {
            enabled,
            history_seconds,
        } => {
            if enabled.is_some() && history_seconds.is_some() {
                return Err("Change one monitoring setting per call".into());
            }
            if let Some(seconds) = history_seconds {
                app.set_cpu_load_history_seconds(*seconds)?;
            }
            if let Some(enabled) = enabled {
                app.set_cpu_load_enabled(*enabled)?;
            }
        }
        _ => return Err("Unsupported state mutation".into()),
    }
    Ok(())
}
pub(super) fn record_graph(
    ctx: &egui::Context,
    data: &super::runtime::GraphData<'_>,
    cpus: &[usize],
    cpu: Option<usize>,
) {
    if debug_mcp::wants_capture(ctx) {
        debug_mcp::graph_capture(
            ctx,
            rendered_graph(data, cpus, cpu, debug_mcp::wants_series(ctx)),
        );
    }
}
pub(super) fn record_contributors(
    ctx: &egui::Context,
    view: &super::contributors::ContributorView,
    cpu: usize,
    order: &[usize],
) {
    if let Some((offset, limit)) = debug_mcp::table_page(ctx, "contributors") {
        debug_mcp::table_capture(
            ctx,
            json!({"kind":"contributors","cpu":cpu,"sample_seconds":view.time.map(|t|t.seconds),"available":view.available,"partial":view.partial,"total":view.rows.len(),"offset":offset,"rows":order.iter().skip(offset).take(limit).map(|&i| {let r=&view.rows[i];json!({"name":r.process.name.as_ref(),"pid":r.process.pid,"instance":r.process.instance,"load":r.load})}).collect::<Vec<_>>()}),
        );
    }
}
fn rendered_graph(
    data: &super::runtime::GraphData<'_>,
    cpus: &[usize],
    cpu: Option<usize>,
    include_series: bool,
) -> Value {
    let tiles:Vec<_>=cpus.iter().map(|&c|json!({"cpu":c,"load":data.sample.values.get(c),"partial":data.sample.partial.get(c),"parked":data.sample.parked.get(c),"activity":data.overview.activity.get(c)})).collect();
    let mut value = json!({"cpu":cpu,"sample_seconds":data.sample.time.map(|t|t.seconds),"inspected_seconds":data.hovered.map(|t|t.seconds),"pending":data.pending,"available":data.available,"tiles":tiles,"series_points":data.series.points.len()});
    if include_series {
        value["series"]=json!(data.series.points.iter().map(|p|json!({"seconds":p.seconds,"load":p.value,"partial":p.partial,"parked":p.parked})).collect::<Vec<_>>());
    }
    value
}
pub(super) fn record_processes(
    ctx: &egui::Context,
    list: &super::process_list::ProcessList,
    order: &[usize],
) {
    let Some((offset, limit)) = debug_mcp::table_page(ctx, "processes") else {
        return;
    };
    let rows = || order.iter().map(|&i| &list.rows[i]);
    debug_mcp::table_capture(
        ctx,
        json!({"kind":"processes","ready":list.ready,"partial":list.partial,"scope":if list.group{"displayed_cpus"}else{"application"},"cpu_percent_basis":if list.group{"displayed_cpu_capacity"}else{"whole_machine"},"total":rows().count(),"offset":offset,"rows":rows().skip(offset).take(limit).map(|r|json!({"name":r.name(),"pid":r.pid,"instance":r.token,"cpu_percent":r.total,"partial":r.partial,"cpus":r.cores,"known_cpus":r.known_cores,"cpu_count":r.cpu_count})).collect::<Vec<_>>()}),
    );
}
