use super::*;
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    source: String,
    group_id: Option<String>,
    rule_id: Option<String>,
    run_id: Option<String>,
    cpu: usize,
    offset: usize,
    limit: usize,
    epoch: Option<String>,
    revision: Option<String>,
    since_revision: String,
    seconds: Option<f64>,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            source: "tracked".into(),
            group_id: None,
            rule_id: None,
            run_id: None,
            cpu: 0,
            offset: 0,
            limit: 60,
            epoch: None,
            revision: None,
            since_revision: "0".into(),
            seconds: None,
        }
    }
}
impl HistoryStore {
    pub(in crate::app::features::cpu_load) fn api_native_interval(
        &mut self,
        time: SampleTime,
        id: u64,
        begin: i64,
        end: i64,
    ) {
        if let Some(row) = self.sample_index(LoadSource::System, time, 0) {
            self.intervals[row] = Some((id, begin, end));
        }
    }
    pub(in crate::app::features::cpu_load) fn api_query(
        &self,
        name: &str,
        args: Value,
    ) -> Result<Value, String> {
        let q: Query = serde_json::from_value(args).map_err(|e| e.to_string())?;
        if q.limit == 0 || q.limit > 256 || q.offset > 100000 || q.cpu >= self.cpus {
            return Err("Invalid CPU/page bounds or CPU not recorded".into());
        }
        let epoch = self.api_epoch.to_string();
        if q.epoch.as_ref().is_some_and(|e| e != &epoch) {
            return Err("cpu_epoch_reset: restart pagination".into());
        }
        if q.revision
            .as_ref()
            .is_some_and(|r| r != &self.revision.to_string())
        {
            return Err("revision_conflict: restart pagination".into());
        }
        let since = q
            .since_revision
            .parse::<u64>()
            .map_err(|_| "Invalid since_revision")?;
        if since > self.revision {
            return Err("cursor_reset: CPU revision is from another history".into());
        }
        let rule = match (&q.group_id, &q.rule_id) {
            (Some(g), Some(r)) => Some(RuleTarget {
                group: crate::app::shared::ids::GroupId(g.clone()),
                rule: crate::app::shared::ids::RuleId(r.clone()),
            }),
            _ => None,
        };
        let source = match q.source.as_str() {
            "system" => LoadSource::System,
            "tracked" => LoadSource::Tracked,
            "run" => LoadSource::Run(q.run_id.as_deref().ok_or("run source requires run_id")?),
            "rule" => LoadSource::Rule(
                rule.as_ref()
                    .ok_or("rule scope needs group_id and rule_id")?,
            ),
            _ => return Err("Unknown CPU source".into()),
        };
        if name == "read_cpu_contributors" {
            let seconds = q
                .seconds
                .ok_or("An exact original seconds value is required")?;
            let mut sample = HistoricalSample::default();
            self.copy_sample(source, seconds, &mut sample);
            let time = sample
                .time
                .ok_or("sample_expired: original interval is not retained")?;
            let mut view = super::super::contributors::ContributorView::default();
            let scope = match source {
                LoadSource::System => super::super::contributors::ContributorScope::System,
                LoadSource::Tracked => super::super::contributors::ContributorScope::Tracked,
                LoadSource::Rule(t) => super::super::contributors::ContributorScope::Rule(&t.rule),
                LoadSource::Run(id) => super::super::contributors::ContributorScope::Run(id),
            };
            self.contributors.copy(time, q.cpu, scope, &mut view);
            let rows:Vec<_>=view.rows.iter().skip(q.offset).take(q.limit).map(|r|json!({"pid":r.process.pid,"historical_lifetime_id":r.process.instance.to_string(),"native_creation_token":null,"name":r.process.name,"load":r.load})).collect();
            return Ok(
                json!({"epoch":epoch,"revision":self.revision.to_string(),"seconds":seconds,"cpu":q.cpu,"available":view.available,"partial":view.partial,"total":view.rows.len(),"rows":rows,"identity_warning":"Historical lifetime IDs cannot authorize live process control"}),
            );
        }
        let data = self.source(source);
        let mut rows = Vec::new();
        let mut matched = 0;
        for position in 0..self.len {
            let row = self.index(position);
            if name == "read_cpu_changes" && self.row_revisions[row] <= since {
                continue;
            }
            if matched < q.offset {
                matched += 1;
                continue;
            }
            if rows.len() >= q.limit {
                break;
            }
            matched += 1;
            let time = self.times[row];
            let cell = row * self.cpus + q.cpu;
            let (load, partial) = data
                .filter(|(_, added)| !added.is_some_and(|added| time.seconds <= added))
                .map(|(loads, _)| (loads.read(cell), loads.partial(cell)))
                .unwrap_or((None, None));
            let frequency = os_api::cpu_load::history_frequency().ok();
            let interval=self.intervals[row].map(|(id,begin,end)|json!({"native_interval_id":id.to_string(),"begin_qpc":begin.to_string(),"end_qpc":end.to_string(),"qpc_frequency":frequency.map(|f|f.to_string()),"duration_seconds":frequency.map(|f|(end as f64-begin as f64)/f as f64)}));
            rows.push(json!({"seconds":time.seconds,"unix_ms":time.wall.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis().to_string(),"row_revision":self.row_revisions[row].to_string(),"interval":interval,"delivery":match self.states[row]{0=>"pending",1=>"completed",_=>"lost"},"gap_reason":self.gaps[row].map(GapReason::code),"load":load,"measured_subtotal":partial,"load_quality":if load.is_some(){"complete"}else if partial.is_some(){"partial"}else{"unavailable"},"parked":match self.parking[cell]{1=>Some(false),2=>Some(true),_=>None}}));
        }
        Ok(
            json!({"epoch":epoch,"revision":self.revision.to_string(),"source":q.source,"cpu":q.cpu,"total_retained":self.len,"next_offset":(rows.len()==q.limit).then_some(q.offset+rows.len()),"retained_from_seconds":(self.len>0).then(||self.times[self.index(0)].seconds),"rows":rows,"correction_policy":"Upsert by epoch and original seconds; discard rows older than retained_from_seconds. Page at a fixed revision, then advance since_revision to revision.","metric":"ETW scheduler residency; percent of one logical CPU at original event time. PDH and GetProcessTimes are different metrics"}),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn correction_feed_upserts_the_original_pending_row_and_never_turns_loss_into_zero() {
        let mut store = HistoryStore::default();
        store.configure(1, 30, &[]).unwrap();
        let time = SampleTime {
            seconds: 1.0,
            wall: SystemTime::UNIX_EPOCH,
        };
        store.push_pending(time, &[]);
        let first = store
            .api_query(
                "read_cpu_changes",
                json!({"source":"system","cpu":0,"since_revision":"0"}),
            )
            .unwrap();
        let cursor = first["revision"].as_str().unwrap().to_owned();
        let snapshot = super::super::super::model::Snapshot {
            cpus: vec![Some(25.0)],
            ..Default::default()
        };
        assert!(store.complete(time, &snapshot));
        let corrected = store
            .api_query(
                "read_cpu_changes",
                json!({"source":"system","cpu":0,"since_revision":cursor}),
            )
            .unwrap();
        assert_eq!(corrected["rows"][0]["seconds"], 1.0);
        assert_eq!(corrected["rows"][0]["load"], 25.0);
        let time = SampleTime {
            seconds: 2.0,
            wall: SystemTime::UNIX_EPOCH,
        };
        store.push_pending(time, &[]);
        store.mark_lost(time);
        let lost = store
            .api_query("read_cpu_history", json!({"source":"system","cpu":0}))
            .unwrap();
        assert_eq!(lost["rows"][1]["delivery"], "lost");
        assert!(lost["rows"][1]["load"].is_null());
        for (name, mut data) in [
            ("read_cpu_changes", first),
            ("read_cpu_changes", corrected),
            ("read_cpu_history", lost),
        ] {
            data["scope_key"] = json!("test-scope");
            crate::app::features::mcp::assert_contract(name, &data);
        }
    }
}
