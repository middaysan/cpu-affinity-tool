use super::*;
use serde_json::{json, Value};
impl HistoryStore {
    pub(in crate::app::features::cpu_load) fn debug_latest_completed(&self) -> Option<f64> {
        (0..self.len)
            .rev()
            .map(|i| self.index(i))
            .find(|&i| self.states[i] == 1)
            .map(|i| self.times[i].seconds)
    }
    pub(in crate::app::features::cpu_load) fn debug_duration(&self) -> u32 {
        self.duration
    }
    pub(in crate::app::features::cpu_load) fn debug_quality(
        &self,
        source: LoadSource<'_>,
        selected: Option<f64>,
        cpu: usize,
        offset: usize,
        limit: usize,
    ) -> Value {
        let source = self.source(source);
        let selected = selected.or(self.finalized.map(|t| t.seconds));
        let mut counts = [0usize; 3];
        let mut load_counts = [0usize; 4];
        let mut page = Vec::new();
        let mut chosen = Value::Null;
        for (position, index) in (0..self.len).rev().map(|i| self.index(i)).enumerate() {
            let state = self.states[index];
            counts[state.min(2) as usize] += 1;
            let seconds = self.times[index].seconds;
            let (quality, reason, load, subtotal) = match source {
                None => ("source_not_recorded", "source_not_recorded", None, None),
                Some((_, added)) if added.is_some_and(|t| seconds <= t) => {
                    ("source_not_recorded", "source_not_recorded", None, None)
                }
                _ if state == 0 => ("unavailable", "delivery_pending", None, None),
                _ if state == 2 => ("unavailable", "interval_lost", None, None),
                Some((loads, _)) if cpu < self.cpus => {
                    let cell = index * self.cpus + cpu;
                    if let Some(value) = loads.read(cell) {
                        ("complete", "measured", Some(value), None)
                    } else if let Some(value) = loads.partial(cell) {
                        ("partial", "partial_measurement", None, Some(value))
                    } else {
                        ("unavailable", "measurement_unavailable", None, None)
                    }
                }
                _ => ("unavailable", "cpu_not_recorded", None, None),
            };
            load_counts[match quality {
                "complete" => 0,
                "partial" => 1,
                "source_not_recorded" => 3,
                _ => 2,
            }] += 1;
            if selected == Some(seconds)
                || (position >= offset && position < offset.saturating_add(limit))
            {
                let row = json!({"seconds":seconds,"delivery":match state {0=>"pending",1=>"completed",_=>"lost"},"load_quality":quality,"reason":reason,"gap_reason":self.gaps[index].map(GapReason::code),"load":load,"measured_subtotal":subtotal});
                if selected == Some(seconds) {
                    chosen = row.clone();
                }
                if position >= offset && position < offset.saturating_add(limit) {
                    page.push(row);
                }
            }
        }
        json!({"cpu":cpu,"revision":self.revision,"total":self.len,"offset":offset,"intervals":page,"selected":chosen,"delivery_counts":{"pending":counts[0],"completed":counts[1],"lost":counts[2]},"load_counts":{"complete":load_counts[0],"partial":load_counts[1],"unavailable":load_counts[2],"source_not_recorded":load_counts[3]},"latest_completed_seconds":self.debug_latest_completed(),"note":"Delivery completion is independent of per-CPU measurement completeness. gap_reason retains the original bounded interval failure category; other measurement availability may remain unknown. Current errors are not historical evidence."})
    }
}
