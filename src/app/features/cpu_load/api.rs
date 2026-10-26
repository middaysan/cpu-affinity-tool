use super::{history::HistoryStore, model::Snapshot};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};
#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CurrentPage {
    offset: usize,
    limit: usize,
    cpu_offset: usize,
    cpu_limit: usize,
}
impl Default for CurrentPage {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
            cpu_offset: 0,
            cpu_limit: 64,
        }
    }
}
#[derive(Clone)]
pub(crate) struct CpuReadHandle {
    exports: Arc<super::export::ExportManager>,
    published: Arc<RwLock<Arc<Snapshot>>>,
    history: Arc<RwLock<HistoryStore>>,
}
impl super::CpuLoadRuntime {
    pub(crate) fn read_handle(&self) -> CpuReadHandle {
        CpuReadHandle {
            exports: self.exports.clone(),
            published: self.published.clone(),
            history: self.history.clone(),
        }
    }
}
impl CpuReadHandle {
    #[cfg(test)]
    pub(crate) fn history_export_fixture() -> Self {
        let mut store = HistoryStore::default();
        store.configure(1, 30, &[]).unwrap();
        let wall = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
        store.push(1.0, wall, &[Some(12.5)], &[], &Default::default());
        store.contributors.push(
            super::history::SampleTime { seconds: 1.0, wall },
            &[super::contributors::Contribution {
                process: Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 4242,
                    instance: 7,
                    name: "пример.exe".into(),
                }),
                rule: None,
                run: None,
                cpu: 0,
                load: 12.5,
            }],
            true,
        );
        Self {
            exports: Default::default(),
            published: Default::default(),
            history: Arc::new(RwLock::new(store)),
        }
    }
    pub(crate) fn export(&self, ctx: eframe::egui::Context, args: Value) -> Result<Value, String> {
        self.exports.query(self.history.clone(), ctx, args)
    }
    pub(crate) fn read(&self, name: &str, args: Value) -> Result<Value, String> {
        if name == "read_run_cpu" {
            let id = args
                .get("run_id")
                .and_then(Value::as_str)
                .ok_or("run_id is required")?;
            let s = self
                .published
                .try_read()
                .map_err(|_| "busy: CPU sample publication")?;
            return Ok(
                json!({"run_id":id,"original_scheduler_seconds":s.scheduler_time.map(|t|t.seconds),"original_scheduler_unix_ms":s.scheduler_time.map(|t|t.wall.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().to_string()),"whole_host_cpu_percent":s.run_totals.get(id).copied().flatten(),"etw_residency_percent":s.run_cpus.get(id),"measured_subtotal":s.run_partial_cpus.get(id),"sample_age_ms":s.at.elapsed().as_millis(),"run_telemetry_error":s.run_error,"collector_error":s.collector_error,"error":s.error,"metric":"Exact managed root only; current whole-host GetProcessTimes and finalized original-time ETW residency have independent intervals. Unknown is not zero."}),
            );
        }
        if name == "read_cpu_current" {
            let page: CurrentPage = serde_json::from_value(args).map_err(|e| e.to_string())?;
            if page.limit == 0
                || page.limit > 256
                || page.offset > 100000
                || page.cpu_limit == 0
                || page.cpu_limit > 256
                || page.cpu_offset > 1024
            {
                return Err("Invalid CPU process page".into());
            }
            let s = self
                .published
                .try_read()
                .map_err(|_| "busy: CPU sample publication")?
                .clone();
            let cpu_end = (page.cpu_offset + page.cpu_limit).min(s.cpus.len());
            let cpu_start = page.cpu_offset.min(cpu_end);
            let cpu_slice =
                |values: &[Option<f32>]| values.get(cpu_start..cpu_end).unwrap_or(&[]).to_vec();
            let parked = s.parked.get(cpu_start..cpu_end).unwrap_or(&[]);
            let mut cells = (cpu_end - cpu_start) * 4;
            let rows:Vec<_>=s.live_processes.rows.iter().skip(page.offset).take(page.limit).take_while(|r|{cells+=r.cores.len().max(1);cells<=4096}).map(|r|json!({"pid":r.pid,"creation_token":r.token.to_string(),"name":r.name(),"whole_host_cpu_percent":r.total,"active_cpus":r.cores,"mapping_partial":r.mapping_partial(),"group_id":r.target.as_ref().map(|t|&t.group),"rule_id":r.target.as_ref().map(|t|&t.rule)})).collect();
            let next = page.offset + rows.len();
            return Ok(
                json!({"revision":s.revision.to_string(),"sample_age_ms":s.at.elapsed().as_millis(),"cpu_offset":cpu_start,"total_cpus":s.cpus.len(),"next_cpu_offset":(cpu_end<s.cpus.len()).then_some(cpu_end),"pdh_non_idle_percent":cpu_slice(&s.cpus),"tracked_etw_residency_percent":cpu_slice(&s.tracked_cpus),"tracked_measured_subtotal":cpu_slice(&s.tracked_partial_cpus),"parked":parked,"collector_error":s.collector_error,"error":s.error,"detail_error":s.detail_error,"history_bytes":s.history_bytes,"current_processes":rows,"next_offset":(next<s.live_processes.rows.len()).then_some(next),"process_total":s.live_processes.rows.len(),"metrics":"PDH per-CPU non-idle and GetProcessTimes whole-host process CPU are current counters; ETW is the finalized original scheduler interval"}),
            );
        }
        self.history
            .try_read()
            .map_err(|_| "busy: CPU history publication")?
            .api_query(name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_process_mapping_quality_tracks_unknown_partial_complete_and_recovery() {
        use crate::app::features::cpu_load::{model::RuleTarget, process_list::ProcessList};
        use crate::app::shared::ids::{GroupId, RuleId};
        use os_api::cpu_load::ProcessCpuSample;
        let target = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("r".into()),
        };
        let make_list = || {
            ProcessList::from_tracking(
                &std::collections::HashMap::from([(target.clone(), vec![(42, 100)])]),
                &std::collections::HashMap::from([((42, 100), Some(2.5))]),
                &Default::default(),
            )
        };
        let assert_mapping = |list: ProcessList, partial, load: Option<f32>| {
            let handle = CpuReadHandle {
                exports: Default::default(),
                published: Arc::new(RwLock::new(Arc::new(Snapshot {
                    live_processes: Arc::new(list),
                    ..Default::default()
                }))),
                history: Default::default(),
            };
            let mut data = handle.read("read_cpu_current", json!({})).unwrap();
            let row = &data["current_processes"][0];
            assert_eq!(
                row["mapping_partial"], partial,
                "sparse or missing CPU mapping must not claim completeness: {row}"
            );
            assert_eq!(
                row["whole_host_cpu_percent"],
                json!(load),
                "mapping quality must not replace independent process-time CPU"
            );
            data["api_epoch"] = json!("test-api");
            crate::app::features::mcp::assert_contract("read_cpu_current", &data);
        };
        assert_mapping(make_list(), true, Some(2.5));
        for (cpus, partial) in [
            (vec![Some(3.0), None], true),
            (vec![None, None], true),
            (vec![Some(0.0), Some(0.0)], false),
            (vec![Some(3.0), Some(0.0)], false),
        ] {
            let mut list = make_list();
            list.record_cores(
                2,
                &[ProcessCpuSample {
                    pid: 42,
                    token: 100,
                    cpus,
                }],
            );
            assert_mapping(list, partial, Some(2.5));
        }
        let mut list = make_list();
        list.record_cores(2, &[]);
        assert_mapping(list, true, Some(2.5));
        let mut list = make_list();
        list.record_cores(
            2,
            &[ProcessCpuSample {
                pid: 42,
                token: 100,
                cpus: vec![Some(1.0), None],
            }],
        );
        list.record_cores(
            2,
            &[ProcessCpuSample {
                pid: 42,
                token: 100,
                cpus: vec![Some(0.0), Some(1.0)],
            }],
        );
        assert_mapping(list, false, Some(2.5));
        let mut list = make_list();
        list.record_cores(
            2,
            &[ProcessCpuSample {
                pid: 42,
                token: 100,
                cpus: vec![Some(0.0); 2],
            }],
        );
        list.invalidate();
        assert_mapping(list, true, None);
    }

    #[test]
    fn current_cpu_vectors_are_paged_under_the_numeric_cell_budget() {
        let snapshot = Snapshot {
            cpus: vec![Some(1.0); 1024],
            tracked_cpus: vec![Some(0.0); 1024],
            tracked_partial_cpus: vec![None; 1024],
            parked: vec![None; 1024],
            ..Default::default()
        };
        let handle = CpuReadHandle {
            exports: Default::default(),
            published: Arc::new(RwLock::new(Arc::new(snapshot))),
            history: Default::default(),
        };
        let data = handle
            .read("read_cpu_current", json!({"cpu_limit":256}))
            .unwrap();
        assert_eq!(data["pdh_non_idle_percent"].as_array().unwrap().len(), 256);
        assert_eq!(data["next_cpu_offset"], 256);
        assert_eq!(data["total_cpus"], 1024);
        let mut wire = data;
        wire["api_epoch"] = json!("test-api");
        crate::app::features::mcp::assert_contract("read_cpu_current", &wire);
    }
}
