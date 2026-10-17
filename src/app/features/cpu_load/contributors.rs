use super::history::SampleTime;
use crate::app::shared::ids::RuleId;
use os_api::cpu_load::CpuProcess;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) enum ContributorScope<'a> {
    System,
    Tracked,
    Rule(&'a RuleId),
}
impl<'a> From<Option<&'a RuleId>> for ContributorScope<'a> {
    fn from(rule: Option<&'a RuleId>) -> Self {
        rule.map(Self::Rule).unwrap_or(Self::System)
    }
}

pub(super) struct Contribution {
    pub process: Arc<CpuProcess>,
    pub rule: Option<Arc<RuleId>>,
    pub cpu: usize,
    pub load: f32,
}
pub(super) struct ContributorRow {
    pub process: Arc<CpuProcess>,
    pub load: f32,
    pid_text: String,
    load_text: String,
}
impl ContributorRow {
    pub fn new(process: Arc<CpuProcess>, load: f32) -> Self {
        Self {
            pid_text: process.pid.to_string(),
            load_text: super::view::percent(Some(load)),
            process,
            load,
        }
    }
    pub fn pid_text(&self) -> &str {
        &self.pid_text
    }
    pub fn load_text(&self) -> &str {
        &self.load_text
    }
}
#[derive(Default)]
pub(super) struct ContributorView {
    pub time: Option<SampleTime>,
    pub rows: Vec<ContributorRow>,
    pub available: bool,
    pub partial: bool,
}
impl ContributorView {
    pub fn clear(&mut self) {
        self.time = None;
        self.rows.clear();
        self.available = false;
        self.partial = false;
    }
}
#[derive(Clone, Copy, Default)]
struct Entry {
    identity: u32,
    cpu: u16,
    load: f32,
}
#[derive(Clone, Copy)]
struct Frame {
    time: SampleTime,
    start: usize,
    len: usize,
    available: bool,
    partial: bool,
    system_partial: bool,
    invalid: [u64; 16],
}
struct Identity {
    process: Arc<CpuProcess>,
    rule: Option<Arc<RuleId>>,
    refs: usize,
}
type Key = (u32, u64, Option<Arc<RuleId>>);
const MAX_IDENTITIES: usize = 8192;
// Sparse pool, not a dense PID × CPU × time matrix. Full frames are evicted
// under exceptional churn rather than silently truncating a named process list.
pub(super) fn contributor_bytes(cpus: usize, seconds: u32) -> Option<usize> {
    let frames = seconds as usize + 1;
    let entries = cpus.checked_mul(32)?.max(256).checked_mul(frames)?;
    entries
        .checked_mul(size_of::<Entry>())?
        .checked_add(frames.checked_mul(size_of::<Option<Frame>>())?)
        .and_then(|v| {
            v.checked_add(
                MAX_IDENTITIES * (size_of::<Option<Identity>>() + 128 + size_of::<usize>()),
            )
        })
}
#[derive(Default)]
pub(super) struct ContributorHistory {
    entries: Vec<Entry>,
    frames: Vec<Option<Frame>>,
    identities: Vec<Option<Identity>>,
    lookup: HashMap<Key, usize>,
    free: Vec<usize>,
    entry_head: usize,
    used: usize,
    frame_head: usize,
    len: usize,
    duration: u32,
}
impl ContributorHistory {
    pub fn new(cpus: usize, seconds: u32) -> Result<Self, String> {
        let bytes =
            contributor_bytes(cpus, seconds).ok_or("CPU contributor history size overflow")?;
        if cpus == 0 || cpus > 1024 || bytes > 192 * 1024 * 1024 {
            return Err("CPU contributor history is too large; choose a shorter history".into());
        }
        fn reserve<T: Clone>(n: usize, value: T) -> Result<Vec<T>, String> {
            let mut values = Vec::new();
            values
                .try_reserve_exact(n)
                .map_err(|_| "Not enough memory for CPU contributors")?;
            values.resize(n, value);
            Ok(values)
        }
        let mut identities = Vec::new();
        identities
            .try_reserve_exact(MAX_IDENTITIES)
            .map_err(|_| "Not enough memory for CPU identities")?;
        let mut lookup = HashMap::new();
        lookup
            .try_reserve(MAX_IDENTITIES)
            .map_err(|_| "Not enough memory for CPU identities")?;
        let mut free = Vec::new();
        free.try_reserve_exact(MAX_IDENTITIES)
            .map_err(|_| "Not enough memory for CPU identities")?;
        Ok(Self {
            entries: reserve(
                cpus.saturating_mul(32).max(256) * (seconds as usize + 1),
                Entry::default(),
            )?,
            frames: reserve(seconds as usize + 1, None)?,
            identities,
            lookup,
            free,
            duration: seconds,
            ..Self::default()
        })
    }
    fn index(&self, offset: usize) -> usize {
        (self.frame_head + self.frames.len() - self.len + offset) % self.frames.len()
    }
    fn evict(&mut self) {
        if self.len == 0 {
            return;
        }
        let index = self.index(0);
        let frame = self.frames[index].take().unwrap();
        for offset in 0..frame.len {
            let entry = self.entries[(frame.start + offset) % self.entries.len()];
            let id = self.identities[entry.identity as usize].as_mut().unwrap();
            id.refs -= 1;
            if id.refs == 0 {
                self.lookup
                    .remove(&(id.process.pid, id.process.instance, id.rule.clone()));
                self.identities[entry.identity as usize] = None;
                self.free.push(entry.identity as usize);
            }
        }
        self.used -= frame.len;
        self.len -= 1;
    }
    pub fn push(&mut self, time: SampleTime, inputs: &[Contribution], available: bool) {
        if self.frames.is_empty() {
            return;
        }
        let count = inputs
            .iter()
            .filter(|v| v.load.is_finite() && v.load > 0.0)
            .count();
        let needed = if available && count <= self.entries.len() {
            count
        } else {
            0
        };
        while self.len > 0
            && (self.len == self.frames.len()
                || time.seconds - self.frames[self.index(0)].unwrap().time.seconds
                    > self.duration as f64
                || self.entries.len() - self.used < needed)
        {
            self.evict();
        }
        let mut frame = Frame {
            time,
            start: self.entry_head,
            len: 0,
            available,
            partial: false,
            system_partial: false,
            invalid: [0; 16],
        };
        for value in inputs
            .iter()
            .filter(|v| v.rule.is_none() && !v.load.is_finite() && v.cpu < 1024)
        {
            frame.invalid[value.cpu / 64] |= 1 << (value.cpu % 64);
        }
        if available && count <= self.entries.len() {
            for value in inputs
                .iter()
                .filter(|v| v.load.is_finite() && v.load > 0.0 && v.cpu <= 1023)
            {
                let key = (
                    value.process.pid,
                    value.process.instance,
                    value.rule.clone(),
                );
                let index = if let Some(&index) = self.lookup.get(&key) {
                    index
                } else {
                    let index = self.free.pop().unwrap_or(self.identities.len());
                    if index >= MAX_IDENTITIES {
                        frame.partial = true;
                        continue;
                    }
                    let identity = Some(Identity {
                        process: value.process.clone(),
                        rule: value.rule.clone(),
                        refs: 0,
                    });
                    if index == self.identities.len() {
                        self.identities.push(identity);
                    } else {
                        self.identities[index] = identity;
                    }
                    self.lookup.insert(key, index);
                    index
                };
                self.identities[index].as_mut().unwrap().refs += 1;
                self.entries[self.entry_head] = Entry {
                    identity: index as u32,
                    cpu: value.cpu as u16,
                    load: value.load.clamp(0.0, 100.0),
                };
                self.entry_head = (self.entry_head + 1) % self.entries.len();
                self.used += 1;
                frame.len += 1;
            }
        } else if count > self.entries.len() {
            frame.available = false;
        }
        self.frames[self.frame_head] = Some(frame);
        self.frame_head = (self.frame_head + 1) % self.frames.len();
        self.len += 1;
    }
    pub fn mark_system_partial(&mut self) {
        if self.len > 0 {
            let index = (self.frame_head + self.frames.len() - 1) % self.frames.len();
            self.frames[index].as_mut().unwrap().system_partial = true;
        }
    }
    pub fn copy<'a>(
        &self,
        time: SampleTime,
        cpu: usize,
        rule: impl Into<ContributorScope<'a>>,
        view: &mut ContributorView,
    ) {
        view.clear();
        let scope = rule.into();
        let (mut low, mut high) = (0, self.len);
        while low < high {
            let mid = (low + high) / 2;
            if self.frames[self.index(mid)].unwrap().time.seconds < time.seconds {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if low == self.len {
            return;
        }
        let frame = self.frames[self.index(low)].unwrap();
        if frame.time != time {
            return;
        }
        view.time = Some(time);
        view.available = frame.available
            && !(matches!(scope, ContributorScope::System)
                && cpu < 1024
                && frame.invalid[cpu / 64] & (1 << (cpu % 64)) != 0);
        view.partial =
            frame.partial || (matches!(scope, ContributorScope::System) && frame.system_partial);
        if !view.available {
            return;
        }
        for offset in 0..frame.len {
            let entry = self.entries[(frame.start + offset) % self.entries.len()];
            if entry.cpu as usize != cpu {
                continue;
            }
            let identity = self.identities[entry.identity as usize].as_ref().unwrap();
            if match scope {
                ContributorScope::System => identity.rule.is_none(),
                ContributorScope::Tracked => identity.rule.is_some(),
                ContributorScope::Rule(rule) => identity.rule.as_deref() == Some(rule),
            } {
                view.rows
                    .push(ContributorRow::new(identity.process.clone(), entry.load));
            }
        }
        if matches!(scope, ContributorScope::Tracked) {
            view.rows.sort_unstable_by(|a, b| {
                a.process
                    .pid
                    .cmp(&b.process.pid)
                    .then(a.process.instance.cmp(&b.process.instance))
                    .then(b.load.total_cmp(&a.load))
            });
            view.rows.dedup_by(|a, b| {
                a.process.pid == b.process.pid && a.process.instance == b.process.instance
            });
        }
        view.rows.sort_unstable_by(|a, b| {
            b.load
                .total_cmp(&a.load)
                .then_with(|| a.process.pid.cmp(&b.process.pid))
                .then_with(|| a.process.instance.cmp(&b.process.instance))
        });
    }
    pub fn resized(&self, cpus: usize, seconds: u32) -> Result<Self, String> {
        let mut next = Self::new(cpus, seconds)?;
        let mut inputs = Vec::new();
        for offset in 0..self.len {
            let frame = self.frames[self.index(offset)].unwrap();
            inputs.clear();
            for offset in 0..frame.len {
                let entry = self.entries[(frame.start + offset) % self.entries.len()];
                if entry.cpu as usize >= cpus {
                    continue;
                }
                let identity = self.identities[entry.identity as usize].as_ref().unwrap();
                inputs.push(Contribution {
                    process: identity.process.clone(),
                    rule: identity.rule.clone(),
                    cpu: entry.cpu as usize,
                    load: entry.load,
                });
            }
            next.push(frame.time, &inputs, frame.available);
            let index = (next.frame_head + next.frames.len() - 1) % next.frames.len();
            let copied = next.frames[index].as_mut().unwrap();
            copied.partial |= frame.partial;
            copied.system_partial = frame.system_partial;
            copied.invalid = frame.invalid;
        }
        Ok(next)
    }
    pub fn metadata_count(&self) -> usize {
        self.lookup.len()
    }
    pub fn reserved_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.frames.capacity() * size_of::<Option<Frame>>()
            + self.identities.capacity() * size_of::<Option<Identity>>()
            + self.lookup.capacity() * 64
            + self.free.capacity() * size_of::<usize>()
    }
    pub fn metadata_bytes(&self) -> usize {
        self.identities
            .iter()
            .flatten()
            .map(|id| id.process.name.len() + size_of::<CpuProcess>() + 32)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::SystemTime;
    fn process(pid: u32, instance: u64, name: &str) -> Arc<os_api::cpu_load::CpuProcess> {
        Arc::new(os_api::cpu_load::CpuProcess {
            pid,
            instance,
            name: name.into(),
        })
    }
    #[test]
    fn tracked_scope_retains_sample_membership_and_distinguishes_reused_pids() {
        let mut history = ContributorHistory::new(1, 30).unwrap();
        let time = SampleTime {
            seconds: 1.0,
            wall: SystemTime::UNIX_EPOCH,
        };
        let a = Arc::new(RuleId("a".into()));
        let b = Arc::new(RuleId("b".into()));
        let row = |token, load, rule| Contribution {
            process: process(42, token, "tracked.exe"),
            rule,
            cpu: 0,
            load,
        };
        history.push(
            time,
            &[
                row(10, 5.0, Some(a.clone())),
                row(10, 5.0, Some(b)),
                row(20, 8.0, Some(a)),
                row(30, 30.0, None),
            ],
            true,
        );
        history.push(
            SampleTime {
                seconds: 2.0,
                ..time
            },
            &[row(10, 30.0, None)],
            true,
        );
        let mut view = ContributorView::default();
        history.copy(time, 0, ContributorScope::Tracked, &mut view);
        assert_eq!(
            view.rows
                .iter()
                .map(|row| (row.process.instance, row.load))
                .collect::<Vec<_>>(),
            vec![(20, 8.0), (10, 5.0)]
        );
        history.copy(
            SampleTime {
                seconds: 2.0,
                ..time
            },
            0,
            ContributorScope::Tracked,
            &mut view,
        );
        assert!(
            view.available && view.rows.is_empty(),
            "a PID with no tracked membership at this sample stays excluded"
        );
        let resized = history.resized(1, 300).unwrap();
        resized.copy(time, 0, ContributorScope::Tracked, &mut view);
        assert_eq!(
            view.rows.len(),
            2,
            "retention resize must preserve original tracked membership"
        );
    }

    #[test]
    fn contributor_history_selects_exact_cpu_time_and_historical_rule_membership() {
        let mut history = ContributorHistory::new(2, 30).unwrap();
        let old = process(10, 100, "old.exe");
        let new = process(10, 200, "new.exe");
        let rule = Arc::new(crate::app::shared::ids::RuleId("r".into()));
        let input = vec![
            Contribution {
                process: old.clone(),
                rule: None,
                cpu: 0,
                load: 20.0,
            },
            Contribution {
                process: old.clone(),
                rule: None,
                cpu: 1,
                load: 80.0,
            },
            Contribution {
                process: old.clone(),
                rule: Some(rule.clone()),
                cpu: 0,
                load: 20.0,
            },
        ];
        let first = SampleTime {
            seconds: 1.0,
            wall: SystemTime::UNIX_EPOCH,
        };
        history.push(first, &input, true);
        history.mark_system_partial();
        let bytes = history.reserved_bytes();
        history.push(
            SampleTime {
                seconds: 2.0,
                ..first
            },
            &[Contribution {
                process: new,
                rule: None,
                cpu: 0,
                load: 90.0,
            }],
            true,
        );
        let mut view = ContributorView::default();
        history.copy(first, 0, None, &mut view);
        assert!(view.available);
        assert!(view.partial, "unknown host work must be explicit");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].process.name.as_ref(), "old.exe");
        assert_eq!(view.rows[0].load, 20.0);
        history.copy(first, 1, None, &mut view);
        assert_eq!(view.rows[0].load, 80.0);
        history.copy(first, 0, Some(rule.as_ref()), &mut view);
        assert_eq!(view.rows.len(), 1);
        assert!(
            !view.partial,
            "unrelated unknown host work cannot invalidate an exact app source"
        );
        history.copy(
            SampleTime {
                seconds: 1.5,
                ..first
            },
            0,
            None,
            &mut view,
        );
        assert!(!view.available, "never substitute live/nearest PID data");
        for second in 3..100 {
            history.push(
                SampleTime {
                    seconds: second as f64,
                    ..first
                },
                &[],
                true,
            );
        }
        history.copy(first, 0, None, &mut view);
        assert!(!view.available);
        assert_eq!(
            history.metadata_count(),
            0,
            "exited names released after eviction"
        );
        assert_eq!(
            history.reserved_bytes(),
            bytes,
            "record pool preallocated once"
        );
    }
    #[test]
    fn contributor_pool_is_bounded_sorted_preserves_resize_and_reports_missing() {
        let mut history = ContributorHistory::new(1, 30).unwrap();
        let time = SampleTime {
            seconds: 1.0,
            wall: SystemTime::UNIX_EPOCH,
        };
        let a = process(1, 1, "a.exe");
        let b = process(2, 2, "b.exe");
        history.push(
            time,
            &[
                Contribution {
                    process: a.clone(),
                    rule: None,
                    cpu: 0,
                    load: 5.0,
                },
                Contribution {
                    process: b.clone(),
                    rule: None,
                    cpu: 0,
                    load: 70.0,
                },
            ],
            true,
        );
        let mut view = ContributorView::default();
        history.copy(time, 0, None, &mut view);
        assert_eq!(
            view.rows.iter().map(|r| r.process.pid).collect::<Vec<_>>(),
            [2, 1]
        );
        let mut resized = history.resized(1, 300).unwrap();
        resized.copy(time, 0, None, &mut view);
        assert_eq!(view.rows.len(), 2);
        let unknown = SampleTime {
            seconds: 2.0,
            ..time
        };
        resized.push(
            unknown,
            &[Contribution {
                process: a.clone(),
                rule: None,
                cpu: 0,
                load: f32::NAN,
            }],
            true,
        );
        resized.copy(unknown, 0, None, &mut view);
        assert!(!view.available);
        // An exceptional frame cannot overwrite past records or fabricate a
        // complete list when it exceeds the whole reserved pool.
        let huge: Vec<_> = (0..=history.entries.len())
            .map(|_| Contribution {
                process: a.clone(),
                rule: None,
                cpu: 0,
                load: 1.0,
            })
            .collect();
        history.push(unknown, &huge, true);
        history.copy(unknown, 0, None, &mut view);
        assert!(!view.available);
        history.copy(time, 0, None, &mut view);
        assert!(
            view.available,
            "an unrecordable frame must not evict valid prior history"
        );
        assert!(history.used <= history.entries.len());
    }
}
