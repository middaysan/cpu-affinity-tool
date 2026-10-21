use super::super::export::{ExportLine, ExportRange, ExportRevision};
use super::*;

impl HistoryStore {
    pub(in crate::app::features::cpu_load) fn export_range(&self) -> Result<ExportRange, String> {
        if self.len == 0 {
            return Err("No CPU history recorded. Enable CPU load and wait for samples.".into());
        }
        Ok(ExportRange {
            epoch: self.api_epoch,
            times: (0..self.len).map(|i| self.times[self.index(i)]).collect(),
        })
    }

    /// Copy only a bounded page; serialization and disk writes happen after this lock is released.
    pub(in crate::app::features::cpu_load) fn export_page(
        &self,
        range: &ExportRange,
        time: SampleTime,
        offset: usize,
        revision: Option<ExportRevision>,
    ) -> Result<(Vec<ExportLine>, ExportRevision, bool), String> {
        if range.epoch != self.api_epoch {
            return Err("History was reset during export. Try again.".into());
        }
        let row = self
            .sample_index(LoadSource::System, time, 0)
            .ok_or("History expired during export. Try again.")?;
        let current = (
            self.row_revisions[row],
            self.contributors.export_token(time),
        );
        if revision.is_some_and(|r| r != current) {
            return Err("An interval changed during export. Try again.".into());
        }
        let delivery = match self.states[row] {
            0 => "pending",
            1 => "completed",
            _ => "lost",
        };
        let mut lines = Vec::with_capacity(128);
        let summary_end = self.cpus * 2;
        let mut next = offset;
        while next < summary_end && lines.len() < 128 {
            let tracked = next >= self.cpus;
            let cpu = next % self.cpus;
            let loads = if tracked { &self.tracked } else { &self.system };
            let cell = row * self.cpus + cpu;
            let value = loads.measured(cell);
            lines.push(ExportLine {
                time,
                interval: self.intervals[row],
                delivery,
                gap: self.gaps[row].map(GapReason::code),
                scope: if tracked { "tracked" } else { "system" },
                process: None,
                rule: None,
                run: None,
                cpu,
                value,
                quality: if loads.read(cell).is_some() {
                    "complete"
                } else if value.is_some() {
                    "partial"
                } else {
                    "unavailable"
                },
                attribution: self.contributors.export_quality(time, cpu, !tracked),
                parked: match self.parking[cell] {
                    1 => Some(false),
                    2 => Some(true),
                    _ => None,
                },
            });
            next += 1;
        }
        let more = if next >= summary_end && lines.len() < 128 {
            let (entries, more) =
                self.contributors
                    .export_entries(time, next - summary_end, 128 - lines.len());
            for entry in entries {
                let system = entry.rule.is_none() && entry.run.is_none();
                let quality = self.contributors.export_quality(time, entry.cpu, system);
                lines.push(ExportLine {
                    time,
                    interval: self.intervals[row],
                    delivery,
                    gap: self.gaps[row].map(GapReason::code),
                    scope: if entry.rule.is_some() {
                        "rule"
                    } else if entry.run.is_some() {
                        "run"
                    } else {
                        "system"
                    },
                    process: Some(entry.process),
                    rule: entry.rule,
                    run: entry.run,
                    cpu: entry.cpu,
                    value: (quality != "unavailable").then_some(entry.load),
                    quality,
                    attribution: quality,
                    parked: None,
                });
            }
            more
        } else {
            true
        };
        Ok((lines, current, more))
    }
}
