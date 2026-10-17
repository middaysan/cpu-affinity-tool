use super::accounting::SharedAccounting;
use super::{CpuContribution, CpuProcess};
use std::collections::VecDeque;
use std::sync::Arc;

pub const PENDING_INTERVALS: usize = 16;
const MAX_TARGETS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CpuHistoryGap {
    Baseline,
    TrackedCapacity,
    ContributorCapacity,
    EventLoss,
    EventSchema,
    EventOrder,
    BufferCapacity,
    Clock,
    Topology,
    Configuration,
    TraceUnavailable,
    MissingFrame,
    Other,
}
impl CpuHistoryGap {
    pub fn code(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::TrackedCapacity => "tracked_capacity",
            Self::ContributorCapacity => "contributor_capacity",
            Self::EventLoss => "event_loss",
            Self::EventSchema => "event_schema",
            Self::EventOrder => "event_order",
            Self::BufferCapacity => "buffer_capacity",
            Self::Clock => "clock",
            Self::Topology => "topology",
            Self::Configuration => "configuration",
            Self::TraceUnavailable => "trace_unavailable",
            Self::MissingFrame => "missing_frame",
            Self::Other => "other",
        }
    }
    pub fn from_error(error: &str) -> Self {
        if error.contains("lost scheduler") || error.contains("lost events") {
            Self::EventLoss
        } else if error.contains("order") || error.contains("continuity") {
            Self::EventOrder
        } else if error.contains("schema")
            || error.contains("version")
            || error.contains("Truncated")
            || error.contains("TDH")
        {
            Self::EventSchema
        } else if error.contains("pending intervals")
            || error.contains("pool")
            || error.contains("overflow")
        {
            Self::BufferCapacity
        } else if error.contains("clock") || error.contains("timestamp") {
            Self::Clock
        } else if error.contains("logical CPU") || error.contains("topology") {
            Self::Topology
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Target {
    pub pid: u32,
    pub token: u64,
    pub verified_at: i64,
    pub alive: Option<bool>,
}
#[derive(Clone, Copy)]
pub struct CpuHistoryTarget {
    pub pid: u32,
    pub token: u64,
}
pub struct CpuHistoryValue {
    pub target: u16,
    pub cpu: u16,
    pub load: f32,
}
pub struct CpuHistoryContribution {
    pub process: Arc<CpuProcess>,
    pub cpu: u16,
    pub load: f32,
}
pub struct CpuHistoryFrame {
    pub id: u64,
    pub begin: i64,
    pub end: i64,
    pub available: bool,
    pub gap: Option<CpuHistoryGap>,
    pub partial: bool,
    pub cpus: Vec<Option<f32>>,
    pub targets: Vec<CpuHistoryTarget>,
    pub coverage: Vec<u64>,
    words: usize,
    pub tracked: Vec<CpuHistoryValue>,
    pub contributors: Vec<CpuHistoryContribution>,
}
impl CpuHistoryFrame {
    pub fn known(&self, target: usize, cpu: usize) -> bool {
        cpu < self.cpus.len()
            && self
                .coverage
                .get(target * self.words + cpu / 64)
                .is_some_and(|bits| bits & (1 << (cpu % 64)) != 0)
    }
    pub fn new(cpus: usize) -> Result<Self, String> {
        history_buffer_bytes(cpus).ok_or("Unsupported CPU interval inventory")?;
        Ok(Self {
            id: 0,
            begin: 0,
            end: 0,
            available: false,
            gap: None,
            partial: false,
            cpus: filled(cpus, None)?,
            targets: capacity(MAX_TARGETS)?,
            coverage: capacity(MAX_TARGETS * cpus.div_ceil(64))?,
            words: cpus.div_ceil(64),
            tracked: capacity(entries(cpus))?,
            contributors: capacity(entries(cpus))?,
        })
    }
    fn reset(&mut self) {
        self.available = false;
        self.gap = None;
        self.partial = false;
        self.cpus.fill(None);
        self.targets.clear();
        self.coverage.clear();
        self.tracked.clear();
        self.contributors.clear();
    }
}
struct Cut {
    id: u64,
    at: i64,
    next: Vec<Target>,
    states: Vec<(u32, u64, Option<bool>)>,
}
pub(super) struct Timeline {
    free_cuts: Vec<Cut>,
    cuts: VecDeque<Cut>,
    free: Vec<CpuHistoryFrame>,
    ready: VecDeque<CpuHistoryFrame>,
    current: Vec<Target>,
    previous: Option<i64>,
    busy_previous: Vec<u64>,
    values: Vec<Option<f32>>,
    pids: Vec<u32>,
    contributors: Vec<CpuContribution>,
    cpus: usize,
}
fn entries(cpus: usize) -> usize {
    // Host attribution includes every active process, including startup bursts.
    // Reserve headroom once; exceeding it still yields an explicit gap.
    (cpus * 128).max(256)
}
fn capacity<T>(count: usize) -> Result<Vec<T>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| "Not enough memory for delayed CPU intervals".to_string())?;
    Ok(values)
}
fn filled<T: Clone>(count: usize, value: T) -> Result<Vec<T>, String> {
    let mut values = capacity(count)?;
    values.resize(count, value);
    Ok(values)
}
pub fn history_buffer_bytes(cpus: usize) -> Option<usize> {
    if cpus == 0 || cpus > 1024 {
        return None;
    }
    let per_frame = cpus * std::mem::size_of::<Option<f32>>()
        + MAX_TARGETS * std::mem::size_of::<CpuHistoryTarget>()
        + MAX_TARGETS * cpus.div_ceil(64) * 8
        + entries(cpus)
            * (std::mem::size_of::<CpuHistoryValue>()
                + std::mem::size_of::<CpuHistoryContribution>());
    Some(
        PENDING_INTERVALS
            * (per_frame
                + MAX_TARGETS
                    * (std::mem::size_of::<Target>()
                        + std::mem::size_of::<(u32, u64, Option<bool>)>())
                + 3 * std::mem::size_of::<CpuHistoryFrame>()
                + 2 * std::mem::size_of::<Cut>())
            + MAX_TARGETS
                * (2 * std::mem::size_of::<Target>()
                    + std::mem::size_of::<(u32, u64, Option<bool>)>()
                    + 4)
            + cpus * 16,
    )
}
impl Timeline {
    pub fn new(cpus: usize) -> Result<Self, String> {
        history_buffer_bytes(cpus).ok_or("Unsupported CPU interval inventory")?;
        let mut free = capacity(PENDING_INTERVALS)?;
        let mut free_cuts = capacity(PENDING_INTERVALS)?;
        for _ in 0..PENDING_INTERVALS {
            free.push(CpuHistoryFrame::new(cpus)?);
            free_cuts.push(Cut {
                id: 0,
                at: 0,
                next: capacity(MAX_TARGETS)?,
                states: capacity(MAX_TARGETS)?,
            });
        }
        let mut cuts = VecDeque::new();
        cuts.try_reserve_exact(PENDING_INTERVALS)
            .map_err(|_| "Not enough memory for CPU cuts")?;
        let mut ready = VecDeque::new();
        ready
            .try_reserve_exact(PENDING_INTERVALS)
            .map_err(|_| "Not enough memory for CPU frames")?;
        Ok(Self {
            free_cuts,
            cuts,
            free,
            ready,
            current: capacity(MAX_TARGETS)?,
            previous: None,
            busy_previous: filled(cpus, 0)?,
            values: filled(cpus, None)?,
            pids: capacity(MAX_TARGETS)?,
            contributors: Vec::new(),
            cpus,
        })
    }
    pub fn queue(
        &mut self,
        id: u64,
        at: i64,
        next: &[Target],
        states: &[(u32, u64, Option<bool>)],
    ) -> Result<(), String> {
        if next.len() > MAX_TARGETS || states.len() > MAX_TARGETS {
            return Err("CPU interval target limit exceeded".into());
        }
        if self
            .cuts
            .back()
            .map(|c| c.at)
            .or(self.previous)
            .is_some_and(|old| at <= old)
        {
            return Err("CPU interval clock did not advance".into());
        }
        let mut cut = self
            .free_cuts
            .pop()
            .ok_or("CPU event delivery exceeded 16 pending intervals")?;
        cut.id = id;
        cut.at = at;
        cut.next.clear();
        cut.next.extend_from_slice(next);
        cut.states.clear();
        cut.states.extend_from_slice(states);
        self.cuts.push_back(cut);
        Ok(())
    }
    pub fn due(&self, at: i64) -> bool {
        self.cuts.front().is_some_and(|cut| cut.at <= at)
    }
    pub fn advance(&mut self, state: &mut SharedAccounting, at: i64) {
        while self.due(at) && !state.failed() {
            let cut = self.cuts.pop_front().unwrap();
            state.finish_at(cut.at);
            let Some(mut frame) = self.free.pop() else {
                state.fail("CPU completed interval pool overflow");
                self.free_cuts.push(cut);
                break;
            };
            frame.reset();
            frame.id = cut.id;
            frame.begin = self.previous.unwrap_or(cut.at);
            frame.end = cut.at;
            frame.available = self.previous.is_some();
            frame.gap = (!frame.available).then_some(CpuHistoryGap::Baseline);
            let duration = self
                .previous
                .and_then(|begin| cut.at.checked_sub(begin))
                .filter(|d| *d > 0);
            for cpu in 0..self.cpus {
                if let Some(duration) = duration.filter(|_| {
                    state
                        .first(cpu)
                        .zip(self.previous)
                        .is_some_and(|(first, begin)| first <= begin)
                }) {
                    frame.cpus[cpu] = Some(
                        ((state.busy_total(cpu) - self.busy_previous[cpu]) as f64 / duration as f64
                            * 100.0)
                            .clamp(0.0, 100.0) as f32,
                    );
                }
                self.busy_previous[cpu] = state.busy_total(cpu);
            }
            for (index, target) in self.current.iter().enumerate() {
                frame.targets.push(CpuHistoryTarget {
                    pid: target.pid,
                    token: target.token,
                });
                frame.coverage.resize((index + 1) * frame.words, 0);
                if let Err(error) = state.sample_into(target.pid, &mut self.values) {
                    state.fail(error);
                    break;
                }
                let valid = cut
                    .states
                    .iter()
                    .find(|(pid, token, _)| *pid == target.pid && *token == target.token)
                    .and_then(|(_, _, alive)| *alive);
                let valid = valid == Some(true)
                    || (valid == Some(false) && state.confirmed_exit(target.pid));
                if valid {
                    for (cpu, value) in self.values.iter().copied().enumerate() {
                        if let Some(value) = value {
                            frame.coverage[index * frame.words + cpu / 64] |= 1 << (cpu % 64);
                            if value > 0.0 {
                                if frame.tracked.len() == frame.tracked.capacity() {
                                    frame.available = false;
                                    frame.gap.get_or_insert(CpuHistoryGap::TrackedCapacity);
                                    break;
                                }
                                frame.tracked.push(CpuHistoryValue {
                                    target: index as u16,
                                    cpu: cpu as u16,
                                    load: value,
                                });
                            }
                        }
                    }
                }
            }
            if let Err(error) = state.sample_contributors(&mut self.contributors) {
                state.fail(error);
            }
            frame.partial = state.contributors_partial();
            for row in &self.contributors {
                for (cpu, value) in row.cpus.iter().enumerate() {
                    if let Some(load) = value.filter(|v| *v > 0.0) {
                        if frame.contributors.len() == frame.contributors.capacity() {
                            frame.available = false;
                            frame.gap.get_or_insert(CpuHistoryGap::ContributorCapacity);
                            break;
                        }
                        frame.contributors.push(CpuHistoryContribution {
                            process: row.process.clone(),
                            cpu: cpu as u16,
                            load,
                        });
                    }
                }
            }
            if !frame.available {
                frame.cpus.fill(None);
                frame.coverage.fill(0);
                frame.tracked.clear();
                frame.contributors.clear();
            }
            self.ready.push_back(frame);
            // The cut closes the old membership before admitting the next one.
            for old in &self.current {
                if !cut
                    .next
                    .iter()
                    .any(|next| next.pid == old.pid && next.token == old.token)
                {
                    state.remove_process(old.pid);
                }
            }
            self.pids.clear();
            self.pids
                .extend(cut.next.iter().filter(|t| t.alive.is_some()).map(|t| t.pid));
            state.set_processes(&self.pids);
            for next in &cut.next {
                if next.alive.is_some() {
                    state.confirm_alive(next.pid, next.verified_at);
                    if next.alive == Some(false) {
                        state.confirm_native_exit(next.pid, cut.at);
                    }
                    if !self.current.iter().any(|old| {
                        old.pid == next.pid && old.token == next.token && old.alive.is_some()
                    }) && let Err(error) = state.sample_into(next.pid, &mut self.values)
                    {
                        state.fail(error);
                    }
                }
            }
            self.current.clear();
            self.current.extend_from_slice(&cut.next);
            self.previous = Some(cut.at);
            self.free_cuts.push(cut);
        }
    }
    pub fn exchange(&mut self, frames: &mut Vec<CpuHistoryFrame>) {
        self.free.append(frames);
        while let Some(frame) = self.ready.pop_front() {
            frames.push(frame);
        }
    }
    pub fn bytes(&self) -> usize {
        history_buffer_bytes(self.cpus).unwrap_or_default()
            + self.contributors.capacity() * std::mem::size_of::<CpuContribution>()
            + self
                .contributors
                .iter()
                .map(|row| row.cpus.capacity() * std::mem::size_of::<Option<f32>>())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu19_startup_burst_keeps_eight_hundred_contributions_in_reserved_storage() {
        let mut state = SharedAccounting::new(20);
        state.enable_contributors();
        for cpu in 0..20 {
            state.switch(cpu, 0, 0, 0);
        }
        let mut timeline = Timeline::new(20).unwrap();
        timeline.queue(0, 10, &[], &[]).unwrap();
        timeline.advance(&mut state, 10);
        let mut at = 20;
        for pid in 1..=40 {
            state.process_start(pid, "burst.exe".into(), at, false);
            for cpu in 0..20 {
                let tid = pid * 32 + cpu as u32 + 1;
                state.thread_start(tid, pid, at);
                state.switch(cpu, 0, tid, at);
                state.switch(cpu, tid, 0, at + 1);
                at += 2;
            }
        }
        timeline.queue(1, at + 10, &[], &[]).unwrap();
        timeline.advance(&mut state, at + 10);
        let mut frames = Vec::new();
        timeline.exchange(&mut frames);
        assert!(
            frames[1].available,
            "a routine startup burst must fit the reservation: {:?}",
            frames[1].gap
        );
        assert_eq!(frames[1].contributors.len(), 800);
        assert!(frames[1].cpus.iter().all(|v| v.is_some_and(|v| v > 0.0)));
        assert!(
            timeline.bytes() < 4 * 1024 * 1024,
            "native interval storage remains bounded"
        );
    }
    #[test]
    fn cpu19_native_gap_reports_baseline_and_contributor_pressure_without_zero() {
        let mut state = SharedAccounting::new(1);
        state.enable_contributors();
        state.switch(0, 0, 0, 0);
        let mut timeline = Timeline::new(1).unwrap();
        timeline.queue(0, 10, &[], &[]).unwrap();
        timeline.advance(&mut state, 10);
        timeline.queue(1, 1010, &[], &[]).unwrap();
        for pid in 1..=300 {
            let at = 10 + i64::from(pid) * 2;
            state.process_start(pid, "busy.exe".into(), at, false);
            state.thread_start(pid, pid, at);
            state.switch(0, 0, pid, at);
            state.switch(0, pid, 0, at + 1);
        }
        timeline.advance(&mut state, 1010);
        let mut frames = Vec::new();
        timeline.exchange(&mut frames);
        assert_eq!(frames[0].gap, Some(CpuHistoryGap::Baseline));
        assert_eq!(frames[1].gap, Some(CpuHistoryGap::ContributorCapacity));
        assert!(!frames[1].available);
        assert_eq!(frames[1].cpus, [None]);
        frames[1].reset();
        assert_eq!(frames[1].gap, None);
    }
    #[test]
    fn cpu19_failure_categories_do_not_invent_a_specific_cause() {
        assert_eq!(
            CpuHistoryGap::from_error("ETW lost scheduler events or buffers"),
            CpuHistoryGap::EventLoss
        );
        assert_eq!(
            CpuHistoryGap::from_error("ETW events arrived out of timestamp order"),
            CpuHistoryGap::EventOrder
        );
        assert_eq!(
            CpuHistoryGap::from_error("TDH schema unavailable"),
            CpuHistoryGap::EventSchema
        );
        assert_eq!(
            CpuHistoryGap::from_error("unrecognized native failure"),
            CpuHistoryGap::Other
        );
    }
}
