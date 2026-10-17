//! Fixed debug cache, populated only by the existing profiler writer.
use super::*;
use std::collections::VecDeque;
const FRAME_CAPACITY: usize = 4096;
const BUCKET_CAPACITY: usize = 61;
const RESOURCE_CAPACITY: usize = 64;
struct FrameRecord {
    at_us: u64,
    id: u64,
    gui_us: u64,
    passes: usize,
    mode: &'static str,
    backend_ms: Option<f32>,
}
pub(super) struct ResourceRecord {
    pub at_us: u64,
    pub span_s: f64,
    pub one_cpu_percent: f64,
    pub private_bytes: usize,
    pub working_bytes: usize,
}
struct Bucket {
    second: u64,
    stats: [Stat; STAGES],
}
pub(super) struct PerformanceStore {
    frames: VecDeque<FrameRecord>,
    resources: VecDeque<ResourceRecord>,
    buckets: VecDeque<Bucket>,
    evicted_at: Option<u64>,
    last_at: u64,
}
impl PerformanceStore {
    pub(super) fn new() -> Self {
        Self {
            frames: VecDeque::with_capacity(FRAME_CAPACITY),
            resources: VecDeque::with_capacity(RESOURCE_CAPACITY),
            buckets: VecDeque::with_capacity(BUCKET_CAPACITY),
            evicted_at: None,
            last_at: 0,
        }
    }
    fn bucket(&mut self, at: u64) -> Option<&mut [Stat; STAGES]> {
        let second = at / 1_000_000;
        let newest = self.buckets.back().map_or(second, |b| b.second.max(second));
        while self.buckets.front().is_some_and(|b| b.second + 60 < newest) {
            self.buckets.pop_front();
        }
        if second + 60 < newest {
            return None;
        }
        let index = self
            .buckets
            .iter()
            .position(|b| b.second >= second)
            .unwrap_or(self.buckets.len());
        if self.buckets.get(index).is_none_or(|b| b.second != second) {
            self.buckets.insert(
                index,
                Bucket {
                    second,
                    stats: [Stat::default(); STAGES],
                },
            );
        }
        self.buckets.get_mut(index).map(|b| &mut b.stats)
    }
    fn stages(&mut self, at: u64, stats: &[Stat; STAGES]) {
        if let Some(bucket) = self.bucket(at) {
            for (to, from) in bucket.iter_mut().zip(stats) {
                merge(to, from);
            }
        }
    }
    pub(super) fn frame(
        &mut self,
        at_us: u64,
        id: u64,
        stats: &[Stat; STAGES],
        view: &View,
        backend_ms: Option<f32>,
    ) {
        self.stages(at_us, stats);
        self.last_at = self.last_at.max(at_us);
        let gui_us = stats[Stage::GuiLogic as usize]
            .wall_us
            .saturating_add(stats[Stage::GuiUi as usize].wall_us);
        if let Some(last) = self.frames.back_mut().filter(|f| f.id == id) {
            last.at_us = at_us;
            last.gui_us = last.gui_us.saturating_add(gui_us);
            last.passes += 1;
            last.mode = view.mode;
            if backend_ms.is_some() {
                last.backend_ms = backend_ms;
            }
            return;
        }
        if self.frames.len() == FRAME_CAPACITY {
            self.evicted_at = self.frames.pop_front().map(|f| f.at_us);
        }
        self.frames.push_back(FrameRecord {
            at_us,
            id,
            gui_us,
            passes: 1,
            mode: view.mode,
            backend_ms,
        });
    }
    pub(super) fn observe(&mut self, event: &Event) {
        match event {
            Event::Frame {
                at_us,
                data,
                view,
                previous_backend_ms,
                rendered: true,
            } => self.frame(*at_us, data.id, &data.stats, view, *previous_backend_ms),
            Event::Frame { at_us, data, .. } => {
                self.stages(*at_us, &data.stats);
                self.last_at = self.last_at.max(*at_us);
            }
            Event::Span {
                at_us,
                stage,
                wall_us,
                cycles,
            } => {
                if let Some(bucket) = self.bucket(*at_us) {
                    bucket[*stage as usize].add(*wall_us, *cycles);
                }
                self.last_at = self.last_at.max(*at_us);
            }
            _ => {}
        }
    }
    pub(super) fn resource(&mut self, row: ResourceRecord) {
        self.last_at = self.last_at.max(row.at_us);
        if self.resources.len() == RESOURCE_CAPACITY {
            self.resources.pop_front();
        }
        self.resources.push_back(row);
    }
    pub(super) fn snapshot(&self, now: u64, seconds: u32, cpus: usize) -> Value {
        // One-second bins avoid another raw per-stage event history.
        let since = now.saturating_sub(seconds as u64 * 1_000_000) / 1_000_000 * 1_000_000;
        let mut frames: Vec<_> = self
            .frames
            .iter()
            .filter(|f| f.at_us >= since && f.at_us <= now)
            .collect();
        frames.sort_unstable_by_key(|f| f.gui_us);
        let gui: Vec<_> = frames.iter().map(|f| f.gui_us as f64 / 1000.0).collect();
        let mut backend: Vec<_> = frames
            .iter()
            .filter_map(|f| f.backend_ms)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(f64::from)
            .collect();
        backend.sort_unstable_by(f64::total_cmp);
        let rows: Vec<_> = self
            .resources
            .iter()
            .filter(|r| {
                r.at_us <= now && r.at_us as f64 - r.span_s * 1e6 >= since as f64 && r.span_s > 0.0
            })
            .collect();
        let span: f64 = rows.iter().map(|r| r.span_s).sum();
        let cpu = (span > 0.0).then(|| {
            rows.iter()
                .map(|r| r.one_cpu_percent * r.span_s)
                .sum::<f64>()
                / span
        });
        let mut stats = [Stat::default(); STAGES];
        for bucket in self
            .buckets
            .iter()
            .filter(|b| b.second >= since / 1_000_000 && b.second <= now / 1_000_000)
        {
            for (to, from) in stats.iter_mut().zip(&bucket.stats) {
                merge(to, from);
            }
        }
        let mut stages: Vec<_> = stats
            .iter()
            .enumerate()
            .filter(|(_, s)| s.count > 0)
            .collect();
        stages.sort_unstable_by_key(|(_, s)| std::cmp::Reverse(s.wall_us));
        json!({"available":!frames.is_empty()||!rows.is_empty()||!stages.is_empty(),"requested_seconds":seconds,"window_start_us":since,"window_end_us":now,"stage_resolution_seconds":1,"last_observation_age_ms":now.saturating_sub(self.last_at) as f64/1000.0,"cache_reserved_bytes":self.frames.capacity()*std::mem::size_of::<FrameRecord>()+self.resources.capacity()*std::mem::size_of::<ResourceRecord>()+self.buckets.capacity()*std::mem::size_of::<Bucket>(),
            "frames":{"count":frames.len(),"p50_gui_ms":percentile(&gui,50),"p95_gui_ms":percentile(&gui,95),"max_gui_ms":gui.last(),"previous_backend_ms_p95":percentile(&backend,95),"evicted_in_window":self.evicted_at.is_some_and(|at|at>=since),"slowest":frames.iter().rev().take(10).map(|f|json!({"frame":f.id,"at_us":f.at_us,"gui_ms":f.gui_us as f64/1000.0,"passes":f.passes,"view":f.mode})).collect::<Vec<_>>()},
            "resource":{"samples":rows.len(),"sampled_seconds":span,"cpu_one_logical_percent_mean":cpu,"cpu_whole_machine_percent_mean":cpu.filter(|_|cpus>0).map(|v|v/cpus as f64),"private_bytes_latest":rows.last().map(|r|r.private_bytes),"working_set_bytes_latest":rows.last().map(|r|r.working_bytes)},
            "stages":stages.iter().map(|(i,s)|json!({"stage":NAMES[*i],"count":s.count,"wall_us":s.wall_us,"max_us":s.max_us,"cycles":s.cycles,"unknown_cycles":s.unknown_cycles})).collect::<Vec<_>>(),
            "timing":"GUI time sums gui_logic + gui_ui across rendered passes of a frame. Child stages are inclusive; do not sum them. Backend timing belongs to the previous frame; no GPU or OS input latency measurement. Resource means use whole observed intervals; stage edge bins are included."})
    }
}
fn merge(to: &mut Stat, from: &Stat) {
    to.count = to.count.saturating_add(from.count);
    to.wall_us = to.wall_us.saturating_add(from.wall_us);
    to.max_us = to.max_us.max(from.max_us);
    to.cycles = to.cycles.saturating_add(from.cycles);
    to.unknown_cycles = to.unknown_cycles.saturating_add(from.unknown_cycles);
}
fn percentile(values: &[f64], percent: usize) -> Option<f64> {
    (!values.is_empty()).then(|| values[(values.len() * percent).div_ceil(100).saturating_sub(1)])
}
#[cfg(test)]
#[path = "debug_performance_tests.rs"]
mod tests;
