use super::*;

pub(super) struct RunHistory {
    pub id: String,
    pub loads: Loads,
    pub added_after: Option<f64>,
}
impl HistoryStore {
    pub(in crate::app::features::cpu_load) fn configure_runs(
        &mut self,
        ids: &[String],
    ) -> Result<(), String> {
        if ids.len() > 64 {
            return Err("Run telemetry capacity exceeded".into());
        }
        let bytes = history_bytes(self.cpus, self.rules.len() + ids.len(), self.duration)
            .and_then(|v| {
                v.checked_add(super::super::contributors::contributor_bytes(
                    self.cpus,
                    self.duration,
                )?)
            })
            .and_then(|v| v.checked_add(os_api::cpu_load::history_buffer_bytes(self.cpus)?))
            .ok_or("Run telemetry size overflow")?;
        if bytes > MAX_HISTORY_BYTES {
            return Err("Run telemetry exceeds the shared 256 MiB monitoring budget".into());
        }
        let mut added = Vec::new();
        for id in ids {
            if !self.runs.iter().any(|r| &r.id == id) {
                added.push(RunHistory {
                    id: id.clone(),
                    loads: Loads::new(self.cpus * self.times.len())?,
                    added_after: (self.len > 0)
                        .then(|| self.times[self.index(self.len - 1)].seconds),
                });
            }
        }
        let changed = !added.is_empty() || self.runs.iter().any(|r| !ids.contains(&r.id));
        self.runs.retain(|r| ids.contains(&r.id));
        self.runs.extend(added);
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(())
    }
}
