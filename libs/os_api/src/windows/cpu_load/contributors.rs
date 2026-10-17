use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Trace-local process lifetime, independent of a recyclable PID. No kernel
/// addresses, command lines, users or paths are retained in telemetry.
#[derive(Debug)]
pub struct CpuProcess {
    pub pid: u32,
    pub instance: u64,
    pub name: Arc<str>,
}
pub struct CpuContribution {
    pub process: Arc<CpuProcess>,
    pub cpus: Vec<Option<f32>>,
}

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
const MAX_PROCESSES: usize = 4096;

struct Interval {
    process: Arc<CpuProcess>,
    ticks: Vec<u64>,
    ended: bool,
}
pub(super) struct Contributors {
    pub processes: HashMap<u32, usize>,
    slots: Vec<Option<Interval>>,
    instances: HashMap<u64, usize>,
    unknown: Vec<bool>,
    last_unknown: Vec<bool>,
    previous: Option<i64>,
    cpus: usize,
}
impl Contributors {
    pub fn new(cpus: usize) -> Self {
        Self {
            processes: HashMap::new(),
            slots: Vec::new(),
            instances: HashMap::new(),
            unknown: vec![false; cpus],
            last_unknown: vec![false; cpus],
            previous: None,
            cpus,
        }
    }
    pub fn start(&mut self, pid: u32, name: Arc<str>, rundown: bool) -> Result<(), String> {
        if pid == 0 || (rundown && self.processes.contains_key(&pid)) {
            return Ok(());
        }
        self.end(pid);
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.slots.len());
        if slot >= MAX_PROCESSES {
            return Err("CPU contributor process limit reached".into());
        }
        let instance = NEXT_INSTANCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| "CPU contributor identity exhausted")?;
        let interval = Some(Interval {
            process: Arc::new(CpuProcess {
                pid,
                instance,
                name,
            }),
            ticks: vec![0; self.cpus],
            ended: false,
        });
        if slot == self.slots.len() {
            self.slots.push(interval);
        } else {
            self.slots[slot] = interval;
        }
        self.processes.insert(pid, slot);
        self.instances.insert(instance, slot);
        Ok(())
    }
    pub fn end(&mut self, pid: u32) {
        if let Some(slot) = self.processes.remove(&pid)
            && let Some(interval) = &mut self.slots[slot]
        {
            interval.ended = true;
        }
    }
    pub fn charge(&mut self, slot: usize, cpu: usize, ticks: u64) -> Result<(), String> {
        if let Some(interval) = &mut self.slots[slot] {
            interval.ticks[cpu] = interval.ticks[cpu]
                .checked_add(ticks)
                .ok_or("CPU contributor accounting overflow")?;
        }
        Ok(())
    }
    pub fn missing(&mut self, cpu: usize) {
        self.unknown[cpu] = true;
    }
    pub fn partial(&self) -> bool {
        self.last_unknown.iter().any(|v| *v)
    }
    pub fn sample(
        &mut self,
        timestamp: i64,
        first: impl Fn(usize) -> Option<i64>,
        rows: &mut Vec<CpuContribution>,
    ) {
        let previous = self.previous.replace(timestamp);
        let duration = previous
            .and_then(|p| timestamp.checked_sub(p))
            .filter(|d| *d > 0);
        // Reuse rows/cell allocations for unchanged lifetimes. Retired rows can
        // be released only after their final interval has been delivered.
        self.last_unknown.clone_from(&self.unknown);
        self.unknown.fill(false);
        rows.retain(|row| self.instances.contains_key(&row.process.instance));
        for interval in self.slots.iter_mut().flatten() {
            let index = match rows
                .binary_search_by_key(&interval.process.instance, |r| r.process.instance)
            {
                Ok(index) => index,
                Err(index) => {
                    rows.insert(
                        index,
                        CpuContribution {
                            process: interval.process.clone(),
                            cpus: vec![None; self.cpus],
                        },
                    );
                    index
                }
            };
            for (cpu, value) in rows[index].cpus.iter_mut().enumerate() {
                *value = duration
                    .filter(|_| first(cpu).zip(previous).is_some_and(|(f, p)| f <= p))
                    .map(|d| {
                        (interval.ticks[cpu] as f64 / d as f64 * 100.0).clamp(0.0, 100.0) as f32
                    });
                interval.ticks[cpu] = 0;
            }
        }
        for slot in &mut self.slots {
            if let Some(interval) = slot.as_ref().filter(|s| s.ended) {
                self.instances.remove(&interval.process.instance);
                *slot = None;
            }
        }
    }
    pub fn bytes(&self) -> usize {
        self.processes.capacity() * 24
            + self.instances.capacity() * 24
            + self.unknown.capacity()
            + self.last_unknown.capacity()
            + self.slots.capacity() * std::mem::size_of::<Option<Interval>>()
            + self
                .slots
                .iter()
                .flatten()
                .map(|s| s.ticks.capacity() * 8 + s.process.name.len() + 64)
                .sum::<usize>()
    }
}
