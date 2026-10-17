use super::model::RuleTarget;
pub(super) use os_api::cpu_load::CpuHistoryGap as GapReason;
use std::collections::HashMap;
use std::mem::size_of;
use std::time::SystemTime;

const MAX_HISTORY_BYTES: usize = 256 * 1024 * 1024;
#[cfg(all(debug_assertions, feature = "debug-mcp"))]
#[path = "history_debug.rs"]
mod debug;

#[derive(Clone, Copy)]
pub(super) enum LoadSource<'a> {
    System,
    Tracked,
    Rule(&'a RuleTarget),
}
impl<'a> From<Option<&'a RuleTarget>> for LoadSource<'a> {
    fn from(target: Option<&'a RuleTarget>) -> Self {
        target.map(Self::Rule).unwrap_or(Self::System)
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SampleTime {
    pub seconds: f64,
    pub wall: SystemTime,
}

#[derive(Default)]
pub(super) struct HistoricalSample {
    pub time: Option<SampleTime>,
    pub values: Vec<Option<f32>>,
    pub partial: Vec<Option<f32>>,
    pub parked: Vec<Option<bool>>,
}
impl HistoricalSample {
    pub fn clear(&mut self) {
        self.time = None;
        self.values.fill(None);
        self.partial.fill(None);
        self.parked.fill(None);
    }
}

/// A bounded min/max envelope for small tiles; full samples stay in the ring.
#[derive(Default)]
pub(super) struct Overview {
    pub revision: u64,
    pub generation: u64,
    pub cpus: usize,
    pub times: Vec<(f64, f64)>,
    pub values: Vec<Option<(f32, f32)>>,
    pub partial: Vec<bool>,
    /// Positive measured work anywhere in the retained source window, even
    /// when a compressed graph bucket has another missing sample.
    pub activity: Vec<bool>,
}
impl Overview {
    pub fn clear(&mut self) {
        self.times.clear();
        self.values.clear();
        self.partial.clear();
        self.activity.clear();
        self.cpus = 0;
        self.generation = self.generation.wrapping_add(1);
    }
}

#[derive(Clone, Copy)]
pub(super) struct SeriesPoint {
    pub seconds: f64,
    pub timestamp: SystemTime,
    pub value: Option<f32>,
    pub partial: bool,
    pub parked: Option<bool>,
}

#[derive(Default)]
pub(super) struct Series {
    pub points: Vec<SeriesPoint>,
    pub duration_seconds: u32,
    pub revision: u64,
    pub generation: u64,
}

#[cfg(test)]
impl Series {
    pub fn set_duration_seconds(&mut self, seconds: u32) {
        self.duration_seconds = seconds;
    }
    pub fn push_sample(&mut self, seconds: f64, cpus: Vec<Option<f32>>, parked: Vec<Option<bool>>) {
        self.push_sample_at(seconds, SystemTime::now(), cpus, parked);
    }
    pub fn push_sample_at(
        &mut self,
        seconds: f64,
        timestamp: SystemTime,
        cpus: Vec<Option<f32>>,
        parked: Vec<Option<bool>>,
    ) {
        self.points.push(SeriesPoint {
            seconds,
            timestamp,
            value: cpus.first().copied().flatten(),
            partial: false,
            parked: parked.first().copied().flatten(),
        });
        self.revision += 1;
        self.generation += 1;
    }
}

// Separate arrays avoid padding a {f32, validity} cell to eight bytes.
#[derive(Default)]
struct Loads {
    values: Vec<f32>,
    valid: Vec<u8>,
}
impl Loads {
    fn new(cells: usize) -> Result<Self, String> {
        Ok(Self {
            values: reserved(cells, 0.0)?,
            valid: reserved(cells, 0)?,
        })
    }
    fn read(&self, index: usize) -> Option<f32> {
        (self.valid[index] == 1).then_some(self.values[index])
    }
    fn measured(&self, index: usize) -> Option<f32> {
        (self.valid[index] != 0).then_some(self.values[index])
    }
    fn partial(&self, index: usize) -> Option<f32> {
        (self.valid[index] == 2).then_some(self.values[index])
    }
    fn write_partial(&mut self, index: usize, value: Option<f32>) {
        if self.valid[index] != 1 {
            if let Some(value) = value.filter(|v| v.is_finite() && *v > 0.0) {
                self.values[index] = value.min(100.0);
                self.valid[index] = 2;
            }
        }
    }
    fn copy_cell(&mut self, to: usize, source: &Self, from: usize) {
        self.values[to] = source.values[from];
        self.valid[to] = source.valid[from];
    }
    fn write(&mut self, index: usize, value: Option<f32>) {
        let value = value.filter(|v| v.is_finite());
        self.valid[index] = u8::from(value.is_some());
        self.values[index] = value.unwrap_or_default().clamp(0.0, 100.0);
    }
    fn bytes(&self) -> usize {
        self.values.capacity() * size_of::<f32>() + self.valid.capacity()
    }
}

fn reserved<T: Clone>(len: usize, value: T) -> Result<Vec<T>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| "Not enough memory for CPU history".to_string())?;
    values.resize(len, value);
    Ok(values)
}

struct RuleHistory {
    target: RuleTarget,
    loads: Loads,
    added_after: Option<f64>,
}

/// One shared timeline/parking ring and fixed numeric storage per saved rule.
/// Numeric recording overwrites preallocated cells; process-lifetime metadata
/// can allocate when a new contributor/source is first observed.
#[derive(Default)]
pub(super) struct HistoryStore {
    pub contributors: super::contributors::ContributorHistory,
    cpus: usize,
    duration: u32,
    times: Vec<SampleTime>,
    states: Vec<u8>,
    gaps: Vec<Option<GapReason>>,
    pending: usize,
    finalized: Option<SampleTime>,
    parking: Vec<u8>,
    system: Loads,
    tracked: Loads,
    rules: Vec<RuleHistory>,
    head: usize,
    len: usize,
    pub revision: u64,
}

pub(super) fn history_bytes(cpus: usize, rules: usize, seconds: u32) -> Option<usize> {
    let capacity = seconds as usize + 1;
    cpus.checked_mul(capacity)?
        .checked_mul(rules.checked_add(2)?.checked_mul(5)?.checked_add(1)?)?
        .checked_add(capacity.checked_mul(size_of::<SampleTime>() + 2)?)
}

impl HistoryStore {
    pub fn value_at<'a>(
        &self,
        target: impl Into<LoadSource<'a>>,
        time: SampleTime,
        cpu: usize,
    ) -> Option<f32> {
        let target = target.into();
        let index = self.sample_index(target, time, cpu)?;
        self.source(target)?.0.read(index * self.cpus + cpu)
    }
    pub fn has_sample<'a>(
        &self,
        target: impl Into<LoadSource<'a>>,
        time: SampleTime,
        cpu: usize,
    ) -> bool {
        self.sample_index(target.into(), time, cpu).is_some()
    }
    fn sample_index(&self, target: LoadSource<'_>, time: SampleTime, cpu: usize) -> Option<usize> {
        if cpu >= self.cpus {
            return None;
        }
        let (_, added) = self.source(target)?;
        if added.is_some_and(|added| time.seconds <= added) {
            return None;
        }
        let (mut low, mut high) = (0, self.len);
        while low < high {
            let mid = (low + high) / 2;
            if self.times[self.index(mid)].seconds < time.seconds {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if low == self.len || self.times[self.index(low)] != time {
            return None;
        }
        Some(self.index(low))
    }
    fn source(&self, target: LoadSource<'_>) -> Option<(&Loads, Option<f64>)> {
        match target {
            LoadSource::System => Some((&self.system, None)),
            LoadSource::Tracked => Some((&self.tracked, None)),
            LoadSource::Rule(target) => self
                .rules
                .iter()
                .find(|r| &r.target == target)
                .map(|rule| (&rule.loads, rule.added_after)),
        }
    }

    /// Copy exactly one recorded row. A missing/evicted row never falls back
    /// to live data, another rule or the nearest newer row.
    pub fn copy_sample<'a>(
        &self,
        target: impl Into<LoadSource<'a>>,
        seconds: f64,
        sample: &mut HistoricalSample,
    ) {
        sample.clear();
        sample.values.resize(self.cpus, None);
        sample.partial.resize(self.cpus, None);
        sample.parked.resize(self.cpus, None);
        let Some((loads, added_after)) = self.source(target.into()) else {
            return;
        };
        let (mut low, mut high) = (0, self.len);
        while low < high {
            let middle = (low + high) / 2;
            if self.times[self.index(middle)].seconds < seconds {
                low = middle + 1
            } else {
                high = middle
            }
        }
        if low == self.len {
            return;
        }
        let index = self.index(low);
        let time = self.times[index];
        if time.seconds != seconds || added_after.is_some_and(|added| time.seconds <= added) {
            return;
        }
        sample.time = Some(time);
        for cpu in 0..self.cpus {
            let cell = index * self.cpus + cpu;
            sample.values[cpu] = loads.read(cell);
            sample.partial[cpu] = loads.partial(cell);
            sample.parked[cpu] = match self.parking[cell] {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            };
        }
    }

    pub fn copy_overview<'a>(&self, target: impl Into<LoadSource<'a>>, overview: &mut Overview) {
        overview.clear();
        overview.revision = self.revision;
        overview.cpus = self.cpus;
        let Some((loads, added_after)) = self.source(target.into()) else {
            return;
        };
        let first = (0..self.len)
            .find(|&offset| {
                !added_after.is_some_and(|added| self.times[self.index(offset)].seconds <= added)
            })
            .unwrap_or(self.len);
        let count = self.len - first;
        let bins = count.min(64);
        overview.activity.resize(self.cpus, false);
        overview.values.resize(self.cpus * bins, None);
        overview.partial.resize(self.cpus * bins, false);
        for bin in 0..bins {
            let from = first + bin * count / bins;
            let until = first + (bin + 1) * count / bins;
            overview.times.push((
                self.times[self.index(from)].seconds,
                self.times[self.index(until - 1)].seconds,
            ));
            for cpu in 0..self.cpus {
                let mut range: Option<(f32, f32)> = None;
                let mut complete = true;
                for offset in from..until {
                    if offset > from
                        && self.times[self.index(offset)].seconds
                            - self.times[self.index(offset - 1)].seconds
                            > 2.5
                    {
                        complete = false;
                    }
                    let cell = self.index(offset) * self.cpus + cpu;
                    let Some(value) = loads.measured(cell) else {
                        complete = false;
                        continue;
                    };
                    overview.activity[cpu] |= value > 0.0;
                    overview.partial[cpu * bins + bin] |= loads.partial(cell).is_some();
                    range = Some(range.map_or((value, value), |(min, max)| {
                        (min.min(value), max.max(value))
                    }));
                }
                overview.values[cpu * bins + bin] = if complete { range } else { None };
            }
        }
    }
    fn addition_boundary(&self, target: &RuleTarget) -> Option<f64> {
        match self
            .rules
            .iter()
            .find(|rule| rule.target.rule == target.rule)
        {
            Some(rule) => rule.added_after,
            None => (self.len > 0).then(|| self.times[self.index(self.len - 1)].seconds),
        }
    }
    pub fn configure(
        &mut self,
        cpus: usize,
        seconds: u32,
        targets: &[RuleTarget],
    ) -> Result<(), String> {
        let seconds =
            crate::app::models::AppStateStorage::normalize_cpu_load_history_seconds(seconds);
        let bytes = history_bytes(cpus, targets.len(), seconds)
            .and_then(|v| v.checked_add(super::contributors::contributor_bytes(cpus, seconds)?))
            .and_then(|v| v.checked_add(os_api::cpu_load::history_buffer_bytes(cpus)?))
            .ok_or("CPU history size overflow")?;
        if cpus == 0 || cpus > 1024 || bytes > MAX_HISTORY_BYTES {
            return Err(
                "CPU history exceeds the 256 MiB monitoring limit; choose a shorter history".into(),
            );
        }
        if self.cpus == cpus && self.duration == seconds {
            if self.rules.len() == targets.len()
                && self.rules.iter().all(|r| targets.contains(&r.target))
            {
                return Ok(());
            }
            // Reuse unchanged rules, including a rule moved to another group.
            let mut added = Vec::new();
            for target in targets {
                if !self.rules.iter().any(|r| r.target.rule == target.rule) {
                    added.push(RuleHistory {
                        target: target.clone(),
                        loads: Loads::new(cpus * self.times.len())?,
                        added_after: self.addition_boundary(target),
                    });
                }
            }
            let changed = self.rules.len() != targets.len()
                || self.rules.iter().any(|r| !targets.contains(&r.target));
            self.rules.retain_mut(|rule| {
                if let Some(target) = targets.iter().find(|t| t.rule == rule.target.rule) {
                    rule.target = target.clone();
                    true
                } else {
                    false
                }
            });
            self.rules.extend(added);
            if changed {
                self.revision = self.revision.wrapping_add(1);
            }
            return Ok(());
        }
        let capacity = seconds as usize + 1;
        let cells = cpus * capacity;
        let mut next = Self {
            contributors: if self.cpus == cpus {
                self.contributors.resized(cpus, seconds)?
            } else {
                super::contributors::ContributorHistory::new(cpus, seconds)?
            },
            cpus,
            duration: seconds,
            times: reserved(
                capacity,
                SampleTime {
                    seconds: 0.0,
                    wall: SystemTime::UNIX_EPOCH,
                },
            )?,
            parking: reserved(cells, 0)?,
            states: reserved(capacity, 2)?,
            gaps: reserved(capacity, None)?,
            system: Loads::new(cells)?,
            tracked: Loads::new(cells)?,
            rules: targets
                .iter()
                .map(|target| {
                    Ok(RuleHistory {
                        target: target.clone(),
                        loads: Loads::new(cells)?,
                        added_after: self.addition_boundary(target),
                    })
                })
                .collect::<Result<_, String>>()?,
            revision: self.revision.wrapping_add(1),
            ..Self::default()
        };
        if self.cpus == cpus {
            let keep = self.len.min(capacity);
            for offset in 0..keep {
                let old = self.index(self.len - keep + offset);
                next.times[offset] = self.times[old];
                next.states[offset] = self.states[old];
                next.gaps[offset] = self.gaps[old];
                for cpu in 0..cpus {
                    let from = old * cpus + cpu;
                    let to = offset * cpus + cpu;
                    next.system.copy_cell(to, &self.system, from);
                    next.tracked.copy_cell(to, &self.tracked, from);
                    next.parking[to] = self.parking[from];
                }
            }
            for rule in &mut next.rules {
                if let Some(previous) = self
                    .rules
                    .iter()
                    .find(|r| r.target.rule == rule.target.rule)
                {
                    for offset in 0..keep {
                        let old = self.index(self.len - keep + offset);
                        for cpu in 0..cpus {
                            rule.loads.copy_cell(
                                offset * cpus + cpu,
                                &previous.loads,
                                old * cpus + cpu,
                            );
                        }
                    }
                }
            }
            next.len = keep;
            next.head = keep % capacity;
            next.pending = (0..keep).filter(|&i| next.states[i] == 0).count();
            next.finalized = (0..keep)
                .rev()
                .find(|&i| next.states[i] != 0)
                .map(|i| next.times[i]);
            next.trim();
        }
        *self = next;
        Ok(())
    }

    fn index(&self, offset: usize) -> usize {
        (self.head + self.times.len() - self.len + offset) % self.times.len()
    }

    fn trim(&mut self) {
        if self.len == 0 {
            return;
        }
        let end = self.times[self.index(self.len - 1)].seconds;
        while self.len > 0 && end - self.times[self.index(0)].seconds > f64::from(self.duration) {
            let old = self.index(0);
            if self.states[old] == 0 {
                self.pending -= 1;
            }
            if self.finalized == Some(self.times[old]) {
                self.finalized = None;
            }
            self.len -= 1;
        }
    }

    pub fn push(
        &mut self,
        seconds: f64,
        wall: SystemTime,
        system: &[Option<f32>],
        parking: &[Option<bool>],
        rules: &HashMap<RuleTarget, Vec<Option<f32>>>,
    ) {
        if self.times.is_empty() {
            return;
        }
        if self.len == self.times.len() && self.states[self.head] == 0 {
            self.pending -= 1;
        }
        self.times[self.head] = SampleTime { seconds, wall };
        self.states[self.head] = 1;
        self.gaps[self.head] = None;
        self.finalized = Some(self.times[self.head]);
        for cpu in 0..self.cpus {
            let cell = self.head * self.cpus + cpu;
            self.system.write(cell, system.get(cpu).copied().flatten());
            self.tracked.write(cell, None);
            self.parking[cell] = match parking.get(cpu).copied().flatten() {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            };
        }
        for rule in &mut self.rules {
            let values = rules.get(&rule.target);
            for cpu in 0..self.cpus {
                rule.loads.write(
                    self.head * self.cpus + cpu,
                    values.and_then(|v| v.get(cpu)).copied().flatten(),
                );
            }
        }
        self.head = (self.head + 1) % self.times.len();
        self.len = (self.len + 1).min(self.times.len());
        self.trim();
        self.revision = self.revision.wrapping_add(1);
    }

    #[cfg(test)]
    pub fn record_tracked(&mut self, values: &[Option<f32>]) {
        if self.len == 0 {
            return;
        }
        let row = self.index(self.len - 1);
        for cpu in 0..self.cpus {
            self.tracked
                .write(row * self.cpus + cpu, values.get(cpu).copied().flatten());
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn push_pending(&mut self, time: SampleTime, parking: &[Option<bool>]) {
        let previous = self.finalized;
        self.push(time.seconds, time.wall, &[], parking, &HashMap::new());
        if self.len > 0 {
            let row = self.index(self.len - 1);
            self.states[row] = 0;
            self.pending += 1;
            #[cfg(feature = "render-profiling")]
            crate::app::features::diagnostics::render_profile::interval(
                time.seconds,
                "pending",
                0,
                self.cpus,
                self.pending,
                None,
            );
            self.finalized =
                previous.filter(|time| self.sample_index(LoadSource::System, *time, 0).is_some());
        }
    }
    pub fn pending_count(&self) -> usize {
        self.pending
    }
    #[cfg(test)]
    pub(super) fn probe_rows(&self, since: f64, rows: &mut Vec<(SampleTime, u8, usize)>) {
        rows.clear();
        for offset in 0..self.len {
            let row = self.index(offset);
            if self.times[row].seconds >= since {
                let unknown = (0..self.cpus)
                    .filter(|&cpu| self.system.read(row * self.cpus + cpu).is_none())
                    .count();
                rows.push((self.times[row], self.states[row], unknown));
            }
        }
    }
    pub fn latest_finalized(&self) -> Option<SampleTime> {
        self.finalized
    }
    pub fn complete(&mut self, time: SampleTime, snapshot: &super::model::Snapshot) -> bool {
        let Some(row) = self.sample_index(LoadSource::System, time, 0) else {
            return false;
        };
        if self.states[row] != 0 {
            return false;
        }
        for cpu in 0..self.cpus {
            let cell = row * self.cpus + cpu;
            self.system
                .write(cell, snapshot.cpus.get(cpu).copied().flatten());
            self.tracked
                .write(cell, snapshot.tracked_cpus.get(cpu).copied().flatten());
            self.tracked.write_partial(
                cell,
                snapshot.tracked_partial_cpus.get(cpu).copied().flatten(),
            );
        }
        for rule in &mut self.rules {
            if rule.added_after.is_some_and(|added| time.seconds <= added) {
                continue;
            }
            let values = snapshot.process_cpus.get(&rule.target).or_else(|| {
                snapshot
                    .process_cpus
                    .iter()
                    .find(|(key, _)| key.rule == rule.target.rule)
                    .map(|(_, v)| v)
            });
            let partial = snapshot.process_partial_cpus.get(&rule.target).or_else(|| {
                snapshot
                    .process_partial_cpus
                    .iter()
                    .find(|(key, _)| key.rule == rule.target.rule)
                    .map(|(_, v)| v)
            });
            for cpu in 0..self.cpus {
                let cell = row * self.cpus + cpu;
                rule.loads
                    .write(cell, values.and_then(|v| v.get(cpu)).copied().flatten());
                rule.loads
                    .write_partial(cell, partial.and_then(|v| v.get(cpu)).copied().flatten());
            }
        }
        self.states[row] = 1;
        self.pending -= 1;
        #[cfg(feature = "render-profiling")]
        crate::app::features::diagnostics::render_profile::interval(
            time.seconds,
            "complete",
            snapshot.cpus.iter().filter(|v| v.is_some()).count(),
            self.cpus,
            self.pending,
            None,
        );
        if self
            .finalized
            .is_none_or(|previous| time.seconds > previous.seconds)
        {
            self.finalized = Some(time);
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }
    #[cfg(test)]
    pub fn mark_lost(&mut self, time: SampleTime) {
        self.mark_lost_reason(time, GapReason::TraceUnavailable);
    }
    pub fn mark_lost_reason(&mut self, time: SampleTime, reason: GapReason) {
        let Some(row) = self.sample_index(LoadSource::System, time, 0) else {
            return;
        };
        if self.states[row] == 0 {
            self.pending -= 1;
        }
        self.states[row] = 2;
        self.gaps[row].get_or_insert(reason);
        #[cfg(feature = "render-profiling")]
        crate::app::features::diagnostics::render_profile::interval(
            time.seconds,
            "lost",
            0,
            self.cpus,
            self.pending,
            self.gaps[row].map(GapReason::code),
        );
        for cpu in 0..self.cpus {
            let cell = row * self.cpus + cpu;
            self.system.write(cell, None);
            self.tracked.write(cell, None);
            for rule in &mut self.rules {
                rule.loads.write(cell, None);
            }
        }
        if self
            .finalized
            .is_none_or(|previous| time.seconds >= previous.seconds)
        {
            self.finalized = Some(time);
        }
        self.revision = self.revision.wrapping_add(1);
    }

    #[cfg(test)]
    pub fn record_partial(
        &mut self,
        tracked: &[Option<f32>],
        rules: &HashMap<RuleTarget, Vec<Option<f32>>>,
    ) {
        if self.len == 0 {
            return;
        }
        let row = self.index(self.len - 1);
        for cpu in 0..self.cpus {
            self.tracked
                .write_partial(row * self.cpus + cpu, tracked.get(cpu).copied().flatten());
        }
        for rule in &mut self.rules {
            if let Some(values) = rules.get(&rule.target) {
                for cpu in 0..self.cpus {
                    rule.loads
                        .write_partial(row * self.cpus + cpu, values.get(cpu).copied().flatten());
                }
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn reserved_bytes(&self) -> usize {
        self.contributors.reserved_bytes()
            + self.times.capacity() * size_of::<SampleTime>()
            + self.states.capacity()
            + self.gaps.capacity() * size_of::<Option<GapReason>>()
            + self.parking.capacity()
            + self.system.bytes()
            + self.tracked.bytes()
            + self.rules.iter().map(|r| r.loads.bytes()).sum::<usize>()
    }

    /// Only the visible CPU series is copied. The UI reuses its reserved Vec.
    pub fn copy_series<'a>(
        &self,
        target: impl Into<LoadSource<'a>>,
        cpu: usize,
        series: &mut Series,
    ) {
        series.points.clear();
        if series.points.capacity() < self.times.len() {
            series.points.reserve(self.times.len());
        }
        series.duration_seconds = self.duration;
        series.revision = self.revision;
        series.generation = series.generation.wrapping_add(1);
        if cpu >= self.cpus {
            return;
        }
        let Some((loads, added_after)) = self.source(target.into()) else {
            return;
        };
        for offset in 0..self.len {
            let index = self.index(offset);
            if added_after.is_some_and(|added| self.times[index].seconds <= added) {
                continue;
            }
            let cell = index * self.cpus + cpu;
            series.points.push(SeriesPoint {
                seconds: self.times[index].seconds,
                timestamp: self.times[index].wall,
                value: loads.measured(cell),
                partial: loads.partial(cell).is_some(),
                parked: match self.parking[cell] {
                    1 => Some(false),
                    2 => Some(true),
                    _ => None,
                },
            });
        }
    }
}
