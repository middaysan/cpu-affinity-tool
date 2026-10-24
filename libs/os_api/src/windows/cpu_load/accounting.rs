use std::collections::HashMap;

pub(super) fn processor_coordinates(counts: &[u32]) -> Result<Vec<(u16, u32)>, String> {
    if counts.is_empty()
        || counts.len() > u16::MAX as usize
        || counts.iter().any(|&n| n == 0 || n > 64)
    {
        return Err("Windows processor group inventory is unavailable".into());
    }
    Ok(counts
        .iter()
        .enumerate()
        .flat_map(|(group, &count)| (0..count).map(move |cpu| (group as u16, cpu)))
        .collect())
}

pub(super) fn process_percent(
    previous: u64,
    current: u64,
    nanos: u128,
    cpus: usize,
) -> Option<f32> {
    if nanos == 0 || cpus == 0 {
        return None;
    }
    let elapsed = current.checked_sub(previous)?;
    Some(((elapsed as f64 * 100.0 * 100.0) / (nanos as f64 * cpus as f64)).clamp(0.0, 100.0) as f32)
}

pub(super) fn two_u32(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((
        u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?),
        u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?),
    ))
}

/// Microsoft PerfView's kernel parser confirms the ID prefix of Thread v1+
/// and CSwitch v2+; only the bounded versions listed here bypass TDH.
/// https://github.com/microsoft/perfview/blob/main/src/TraceEvent/Parsers/KernelTraceEventParser.cs
/// CSwitch v5 is additionally characterized by the actual Windows 11 28-byte
/// event. Its extra final DWORD is irrelevant to scheduler residency.
pub(super) fn scheduler_pair(
    opcode: u8,
    version: u8,
    bytes: &[u8],
) -> Option<Result<(u32, u32), String>> {
    let minimum = match (opcode, version) {
        (1..=4, 1..=4) => 8,
        (36, 2..=4) => 24,
        (36, 5) => 28,
        _ => return None,
    };
    Some(if bytes.len() < minimum {
        Err(format!(
            "Truncated ETW scheduler opcode {opcode} version {version}: requires {minimum} bytes, received {}",
            bytes.len()
        ))
    } else {
        two_u32(bytes).ok_or_else(|| "Invalid ETW scheduler identifier prefix".into())
    })
}

pub(super) fn event_cpu_index(
    index_flag: bool,
    index: u16,
    number: u8,
    count: usize,
) -> Option<usize> {
    // The byte processor number cannot identify all CPUs on a >256-CPU host.
    let cpu = if index_flag {
        usize::from(index)
    } else if count <= 256 {
        usize::from(number)
    } else {
        return None;
    };
    (cpu < count).then_some(cpu)
}

const MAX_OBSERVED_THREADS: usize = 262_144;

fn observed_duration(previous: Option<i64>, current: i64, first: Option<i64>) -> Option<i64> {
    let previous = previous?;
    // A partial first interval is unknown, even if its counter is zero.
    if first? > previous {
        return None;
    }
    current
        .checked_sub(previous)
        .filter(|duration| *duration > 0)
}

#[derive(Default)]
struct SharedCpu {
    since: Option<i64>,
    first: Option<i64>,
    tid: u32,
    slot: Option<usize>,
    system_slot: Option<usize>,
    busy_total: u64,
}
struct ProcessInterval {
    total: Vec<u64>,
    previous_total: Vec<u64>,
    previous: Option<i64>,
    last_alive: Option<i64>,
    ended: Option<i64>,
}

/// A single scheduler stream accounts each admitted PID separately. All live
/// TID->PID facts are retained so newly tracked, already-running processes do
/// not require restarting ETW or discarding other applications' histories.
/// Intervals end at the last delivered QPC timestamp; delayed buffers are never
/// charged speculatively and then charged again when their events arrive.
pub(super) struct SharedAccounting {
    cpus: Vec<SharedCpu>,
    threads: HashMap<u32, u32>,
    processes: HashMap<u32, usize>,
    slots: Vec<Option<ProcessInterval>>,
    watermark: Option<i64>,
    failure: Option<String>,
    contributors: Option<super::contributors::Contributors>,
    timeline: Option<super::timeline::Timeline>,
}
impl SharedAccounting {
    pub fn new(cpus: usize) -> Self {
        Self {
            cpus: (0..cpus).map(|_| SharedCpu::default()).collect(),
            threads: HashMap::with_capacity(4096),
            processes: HashMap::new(),
            slots: Vec::new(),
            watermark: None,
            failure: None,
            contributors: None,
            timeline: None,
        }
    }
    pub fn fail(&mut self, error: impl Into<String>) {
        if self.failure.is_none() {
            self.failure = Some(error.into());
        }
    }
    pub fn enable_contributors(&mut self) {
        self.contributors = Some(super::contributors::Contributors::new(self.cpus.len()));
    }
    pub fn process_start(
        &mut self,
        pid: u32,
        name: std::sync::Arc<str>,
        timestamp: i64,
        rundown: bool,
    ) {
        self.advance(timestamp);
        for cpu in 0..self.cpus.len() {
            self.settle(cpu, timestamp);
        }
        if let Some(system) = &mut self.contributors {
            if let Err(error) = system.start(pid, name, rundown) {
                self.fail(error);
                return;
            }
            for cpu in &mut self.cpus {
                cpu.system_slot = self
                    .threads
                    .get(&cpu.tid)
                    .and_then(|pid| system.processes.get(pid))
                    .copied();
            }
        }
    }
    pub fn process_end(&mut self, pid: u32, timestamp: i64) {
        self.advance(timestamp);
        for cpu in 0..self.cpus.len() {
            self.settle(cpu, timestamp);
        }
        // An older buffered end may belong to a reused PID. Only an end
        // after verified native liveness can retire this exact admitted slot.
        if let Some(slot) = self.processes.get(&pid).copied()
            && let Some(process) = &mut self.slots[slot]
            && process.last_alive.is_some_and(|alive| timestamp > alive)
        {
            process.ended = Some(timestamp);
            for cpu in &mut self.cpus {
                if cpu.slot == Some(slot) {
                    cpu.slot = None;
                }
            }
        }
        if let Some(system) = &mut self.contributors {
            let slot = system.processes.get(&pid).copied();
            system.end(pid);
            for cpu in &mut self.cpus {
                if slot.is_some() && cpu.system_slot == slot {
                    cpu.system_slot = None;
                }
            }
        }
    }
    pub fn sample_contributors(
        &mut self,
        rows: &mut Vec<super::CpuContribution>,
    ) -> Result<(), String> {
        if let Some(error) = &self.failure {
            rows.clear();
            return Err(error.clone());
        }
        let Some(timestamp) = self.watermark else {
            rows.clear();
            return Ok(());
        };
        for cpu in 0..self.cpus.len() {
            self.settle(cpu, timestamp);
        }
        if let Some(error) = &self.failure {
            rows.clear();
            return Err(error.clone());
        }
        if let Some(system) = &mut self.contributors {
            system.sample(timestamp, |cpu| self.cpus[cpu].first, rows);
        }
        Ok(())
    }
    pub fn contributors_partial(&self) -> bool {
        self.contributors.as_ref().is_some_and(|s| s.partial())
    }
    pub fn failed(&self) -> bool {
        self.failure.is_some()
    }
    pub fn advance(&mut self, timestamp: i64) {
        if timestamp < 0 || self.watermark.is_some_and(|t| timestamp < t) {
            self.fail("ETW events arrived out of timestamp order; CPU history has a gap");
        } else {
            if self
                .timeline
                .as_ref()
                .is_some_and(|timeline| timeline.due(timestamp))
            {
                let mut timeline = self.timeline.take().unwrap();
                timeline.advance(self, timestamp);
                self.timeline = Some(timeline);
            }
            self.watermark = Some(timestamp);
        }
    }
    pub fn enable_timeline(&mut self) -> Result<(), String> {
        self.timeline = Some(super::timeline::Timeline::new(self.cpus.len())?);
        Ok(())
    }
    pub fn queue_cut(
        &mut self,
        id: u64,
        at: i64,
        next: impl AsRef<[super::timeline::Target]>,
        states: impl AsRef<[(u32, u64, Option<bool>)]>,
    ) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.watermark.is_some_and(|time| at <= time) {
            return Err("CPU cut predates delivered events".into());
        }
        self.timeline
            .as_mut()
            .ok_or("CPU timeline is disabled")?
            .queue(id, at, next.as_ref(), states.as_ref())
    }
    pub fn exchange_timeline_frames(
        &mut self,
        frames: &mut Vec<super::timeline::CpuHistoryFrame>,
    ) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        self.timeline
            .as_mut()
            .ok_or("CPU timeline is disabled")?
            .exchange(frames);
        Ok(())
    }
    #[cfg(test)]
    pub fn take_timeline_frames(
        &mut self,
    ) -> Result<Vec<super::timeline::CpuHistoryFrame>, String> {
        let mut frames = Vec::with_capacity(super::timeline::PENDING_INTERVALS);
        self.exchange_timeline_frames(&mut frames)?;
        Ok(frames)
    }
    #[cfg(test)]
    pub fn recycle_timeline_frames(&mut self, mut frames: Vec<super::timeline::CpuHistoryFrame>) {
        self.timeline.as_mut().unwrap().exchange(&mut frames);
    }
    pub fn finish_at(&mut self, timestamp: i64) {
        self.watermark = Some(timestamp);
        for cpu in 0..self.cpus.len() {
            self.settle(cpu, timestamp);
        }
    }
    pub fn first(&self, cpu: usize) -> Option<i64> {
        self.cpus[cpu].first
    }
    pub fn busy_total(&self, cpu: usize) -> u64 {
        self.cpus[cpu].busy_total
    }
    pub fn confirm_native_exit(&mut self, pid: u32, at: i64) {
        if let Some(process) = self
            .processes
            .get(&pid)
            .and_then(|slot| self.slots[*slot].as_mut())
        {
            process.ended = Some(at);
        }
        for cpu in &mut self.cpus {
            if cpu
                .slot
                .is_some_and(|slot| self.slots[slot].as_ref().is_some_and(|p| p.ended.is_some()))
            {
                cpu.slot = None;
            }
        }
    }
    fn settle(&mut self, index: usize, timestamp: i64) {
        let cpu = &mut self.cpus[index];
        if let Some(since) = cpu.since {
            let Some(delta) = timestamp
                .checked_sub(since)
                .and_then(|n| u64::try_from(n).ok())
            else {
                self.fail("ETW scheduler accounting overflow");
                return;
            };
            if cpu.tid != 0 {
                let Some(total) = cpu.busy_total.checked_add(delta) else {
                    self.fail("CPU busy-time overflow");
                    return;
                };
                cpu.busy_total = total;
            }
            if let Some(process) = cpu.slot.and_then(|slot| self.slots[slot].as_mut()) {
                let Some(total) = process.total[index].checked_add(delta) else {
                    self.fail("ETW process accounting overflow");
                    return;
                };
                process.total[index] = total;
            }
            if let (Some(system), Some(slot)) = (&mut self.contributors, cpu.system_slot) {
                if let Err(error) = system.charge(slot, index, delta) {
                    self.fail(error);
                    return;
                }
            } else if cpu.tid != 0
                && delta > 0
                && let Some(system) = &mut self.contributors
            {
                system.missing(index);
            }
            cpu.since = Some(timestamp);
        }
    }
    pub fn set_processes(&mut self, pids: &[u32]) {
        if let Some(timestamp) = self.watermark {
            for cpu in 0..self.cpus.len() {
                self.settle(cpu, timestamp);
            }
        }
        self.processes.retain(|pid, slot| {
            if pids.contains(pid) {
                true
            } else {
                self.slots[*slot] = None;
                false
            }
        });
        for &pid in pids {
            if self.processes.contains_key(&pid) {
                continue;
            }
            let slot = self
                .slots
                .iter()
                .position(Option::is_none)
                .unwrap_or(self.slots.len());
            let process = Some(ProcessInterval {
                total: vec![0; self.cpus.len()],
                previous_total: vec![0; self.cpus.len()],
                previous: None,
                last_alive: None,
                ended: None,
            });
            if slot == self.slots.len() {
                self.slots.push(process);
            } else {
                self.slots[slot] = process;
            }
            self.processes.insert(pid, slot);
        }
        for index in 0..self.cpus.len() {
            self.cpus[index].slot = self
                .threads
                .get(&self.cpus[index].tid)
                .and_then(|pid| self.running_slot(*pid));
        }
    }
    fn running_slot(&self, pid: u32) -> Option<usize> {
        self.processes.get(&pid).copied().filter(|slot| {
            self.slots[*slot]
                .as_ref()
                .is_some_and(|p| p.ended.is_none())
        })
    }
    pub fn confirm_alive(&mut self, pid: u32, timestamp: i64) {
        if let Some(process) = self
            .processes
            .get(&pid)
            .and_then(|slot| self.slots[*slot].as_mut())
        {
            process.last_alive = Some(
                process
                    .last_alive
                    .map_or(timestamp, |previous| previous.max(timestamp)),
            );
        }
    }
    pub fn confirmed_exit(&self, pid: u32) -> bool {
        self.processes
            .get(&pid)
            .and_then(|slot| self.slots[*slot].as_ref())
            .is_some_and(|p| p.ended.is_some())
    }
    pub fn remove_process(&mut self, pid: u32) {
        if let Some(slot) = self.processes.remove(&pid) {
            self.slots[slot] = None;
            for cpu in &mut self.cpus {
                if cpu.slot == Some(slot) {
                    cpu.slot = None;
                }
            }
        }
    }
    pub fn thread_start(&mut self, tid: u32, pid: u32, timestamp: i64) {
        self.advance(timestamp);
        if !self.threads.contains_key(&tid) && self.threads.len() >= MAX_OBSERVED_THREADS {
            self.fail("CPU monitoring observed-thread limit reached");
            return;
        }
        self.threads.insert(tid, pid);
        for index in 0..self.cpus.len() {
            if self.cpus[index].tid == tid {
                self.settle(index, timestamp);
                self.cpus[index].slot = self.running_slot(pid);
                self.cpus[index].system_slot = self
                    .contributors
                    .as_ref()
                    .and_then(|s| s.processes.get(&pid))
                    .copied();
            }
        }
    }
    pub fn thread_end(&mut self, tid: u32, timestamp: i64) {
        self.advance(timestamp);
        self.threads.remove(&tid);
        for index in 0..self.cpus.len() {
            if self.cpus[index].tid == tid {
                self.settle(index, timestamp);
                self.cpus[index].slot = None;
                self.cpus[index].system_slot = None;
            }
        }
    }
    pub fn switch(&mut self, index: usize, old: u32, new: u32, timestamp: i64) {
        self.advance(timestamp);
        let Some(cpu) = self.cpus.get(index) else {
            self.fail("ETW reported an unknown logical CPU");
            return;
        };
        if cpu.since.is_some() && cpu.tid != old {
            self.fail("ETW scheduler continuity was lost");
            return;
        }
        self.settle(index, timestamp);
        let slot = self
            .threads
            .get(&new)
            .and_then(|pid| self.running_slot(*pid));
        let cpu = &mut self.cpus[index];
        cpu.first.get_or_insert(timestamp);
        cpu.since = Some(timestamp);
        cpu.tid = new;
        cpu.slot = slot;
        cpu.system_slot = self
            .contributors
            .as_ref()
            .and_then(|system| {
                self.threads
                    .get(&new)
                    .and_then(|pid| system.processes.get(pid))
            })
            .copied();
    }
    pub fn sample_into(&mut self, pid: u32, values: &mut [Option<f32>]) -> Result<(), String> {
        values.fill(None);
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let Some(timestamp) = self.watermark else {
            return Ok(());
        };
        for cpu in 0..self.cpus.len() {
            self.settle(cpu, timestamp);
        }
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let Some(process) = self
            .processes
            .get(&pid)
            .and_then(|slot| self.slots[*slot].as_mut())
        else {
            return Ok(());
        };
        let previous = process.previous.replace(timestamp);
        for (index, value) in values.iter_mut().enumerate().take(self.cpus.len()) {
            // No scheduler initialization is needed to prove that an already
            // ended exact instance did no work in a wholly subsequent interval.
            *value = if previous.is_some_and(|p| process.ended.is_some_and(|end| end <= p)) {
                observed_duration(previous, timestamp, process.ended).map(|_| 0.0)
            } else {
                observed_duration(previous, timestamp, self.cpus[index].first).map(|duration| {
                    ((process.total[index] - process.previous_total[index]) as f64
                        / duration as f64
                        * 100.0)
                        .clamp(0.0, 100.0) as f32
                })
            };
            process.previous_total[index] = process.total[index];
        }
        Ok(())
    }
    #[cfg(test)]
    fn sample(&mut self) -> Result<Vec<Option<f32>>, String> {
        let mut values = vec![None; self.cpus.len()];
        self.sample_into(1, &mut values)?;
        Ok(values)
    }
    pub fn bookkeeping_bytes(&self) -> usize {
        self.timeline.as_ref().map_or(0, |t| t.bytes())
            + self.contributors.as_ref().map_or(0, |s| s.bytes())
            + self.threads.capacity() * 16
            + self.processes.capacity() * 24
            + self.slots.capacity() * std::mem::size_of::<Option<ProcessInterval>>()
            + self
                .slots
                .iter()
                .flatten()
                .map(|slot| (slot.total.capacity() + slot.previous_total.capacity()) * 8)
                .sum::<usize>()
    }
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }
    #[cfg(test)]
    pub(super) fn sample_diagnostic(&self) -> String {
        format!(
            "pids={} threads={} unobserved_cpus={:?} watermark={:?}",
            self.processes.len(),
            self.threads.len(),
            self.cpus
                .iter()
                .enumerate()
                .filter_map(|(i, cpu)| cpu.first.is_none().then_some(i))
                .collect::<Vec<_>>(),
            self.watermark
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_time_cuts_keep_a_delayed_burst_in_its_original_intervals() {
        let mut state = SharedAccounting::new(1);
        state.enable_contributors();
        state.enable_timeline().unwrap();
        let target = super::super::timeline::Target {
            pid: 101,
            token: 10,
            verified_at: 0,
            alive: Some(true),
        };
        state.process_start(101, "load.exe".into(), 0, false);
        state.thread_start(11, 101, 0);
        state.switch(0, 0, 0, 0);
        for (id, at) in [(0, 10), (1, 110), (2, 210)] {
            state
                .queue_cut(id, at, vec![target], vec![(101, 10, Some(true))])
                .unwrap();
        }
        // All boundaries are registered before delivery of the burst.
        state.switch(0, 0, 11, 60);
        state.switch(0, 11, 0, 160);
        state.advance(220);
        let frames = state.take_timeline_frames().unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].id, 1);
        assert_eq!(frames[1].cpus, [Some(50.0)]);
        assert_eq!(frames[2].cpus, [Some(50.0)]);
        for frame in &frames[1..] {
            assert_eq!(frame.tracked.iter().map(|v| v.load).sum::<f32>(), 50.0);
            assert_eq!(frame.contributors.iter().map(|v| v.load).sum::<f32>(), 50.0);
        }
        state.recycle_timeline_frames(frames);
        assert!(
            state.take_timeline_frames().unwrap().is_empty(),
            "completed cuts publish once"
        );
    }

    #[test]
    fn event_loss_never_completes_a_pending_cut_with_fabricated_zero() {
        let mut state = SharedAccounting::new(1);
        state.enable_contributors();
        state.enable_timeline().unwrap();
        state.queue_cut(0, 10, vec![], vec![]).unwrap();
        state.queue_cut(1, 110, vec![], vec![]).unwrap();
        state.advance(50);
        state.fail("lost scheduler events");
        state.advance(200);
        assert!(state.take_timeline_frames().is_err());
    }

    #[test]
    fn recovered_admission_primes_the_first_complete_original_interval() {
        use super::super::timeline::Target;
        let mut state = SharedAccounting::new(1);
        state.enable_contributors();
        state.enable_timeline().unwrap();
        state.process_start(101, "load.exe".into(), 0, false);
        state.thread_start(11, 101, 0);
        state.switch(0, 0, 0, 0);
        let target = Target {
            pid: 101,
            token: 10,
            verified_at: 0,
            alive: None,
        };
        state.queue_cut(0, 10, [target], []).unwrap();
        state
            .queue_cut(
                1,
                110,
                [Target {
                    alive: Some(true),
                    ..target
                }],
                [(101, 10, None)],
            )
            .unwrap();
        state
            .queue_cut(
                2,
                210,
                [Target {
                    alive: Some(true),
                    ..target
                }],
                [(101, 10, Some(true))],
            )
            .unwrap();
        state.switch(0, 0, 11, 160);
        state.switch(0, 11, 0, 210);
        let frames = state.take_timeline_frames().unwrap();
        assert!(!frames[1].known(0, 0));
        assert!(
            frames[2].known(0, 0),
            "newly verifiable same-token target needs a baseline at admission"
        );
        assert_eq!(frames[2].tracked[0].load, 50.0);
    }

    #[test]
    fn delivery_timing_does_not_change_original_frames_or_reused_pid_ownership() {
        use super::super::timeline::Target;
        type FrameSnapshot = (u64, Option<f32>, Vec<(u64, f32)>);
        fn run(delayed: bool) -> Vec<FrameSnapshot> {
            let mut state = SharedAccounting::new(1);
            state.enable_contributors();
            state.enable_timeline().unwrap();
            state.process_start(101, "old.exe".into(), 0, false);
            state.thread_start(11, 101, 0);
            state.switch(0, 0, 0, 0);
            let old = Target {
                pid: 101,
                token: 10,
                verified_at: 0,
                alive: Some(true),
            };
            let new = Target {
                token: 20,
                verified_at: 110,
                ..old
            };
            state.queue_cut(0, 10, [old], []).unwrap();
            if delayed {
                state
                    .queue_cut(1, 110, [new], [(101, 10, Some(true))])
                    .unwrap();
                state
                    .queue_cut(2, 210, [], [(101, 20, Some(true))])
                    .unwrap();
            }
            state.switch(0, 0, 11, 60);
            if !delayed {
                state
                    .queue_cut(1, 110, [new], [(101, 10, Some(true))])
                    .unwrap();
            }
            state.switch(0, 11, 0, 110);
            // Buffered old ProcessEnd must not retire the new verified token.
            state.process_end(101, 110);
            state.process_start(101, "new.exe".into(), 111, false);
            if !delayed {
                state
                    .queue_cut(2, 210, [], [(101, 20, Some(true))])
                    .unwrap();
            }
            state.switch(0, 0, 11, 160);
            state.switch(0, 11, 0, 210);
            assert!(!state.failed());
            let frames = state.take_timeline_frames().unwrap();
            let result = frames
                .iter()
                .map(|f| {
                    (
                        f.id,
                        f.cpus[0],
                        f.tracked
                            .iter()
                            .map(|v| (f.targets[v.target as usize].token, v.load))
                            .collect(),
                    )
                })
                .collect();
            assert_eq!(frames[1].tracked[0].load, 50.0);
            assert_eq!(frames[2].tracked[0].load, 50.0);
            assert_eq!(frames[1].targets[0].token, 10);
            assert_eq!(frames[2].targets[0].token, 20);
            result
        }
        assert_eq!(run(false), run(true));
    }

    #[test]
    fn pending_and_completed_interval_pools_are_bounded_and_reused() {
        let mut state = SharedAccounting::new(1);
        state.enable_contributors();
        state.enable_timeline().unwrap();
        state.switch(0, 0, 0, 0);
        let bytes = state.bookkeeping_bytes();
        for id in 0..16 {
            state.queue_cut(id, (id as i64 + 1) * 10, [], []).unwrap();
        }
        assert!(state.queue_cut(16, 170, [], []).is_err());
        state.advance(160);
        let mut frames = state.take_timeline_frames().unwrap();
        assert_eq!(frames.len(), 16);
        let pointers = frames.iter().map(|f| f.cpus.as_ptr()).collect::<Vec<_>>();
        state.exchange_timeline_frames(&mut frames).unwrap();
        for id in 16..32 {
            state.queue_cut(id, (id as i64 + 1) * 10, [], []).unwrap();
        }
        state.advance(320);
        state.exchange_timeline_frames(&mut frames).unwrap();
        assert_eq!(frames.len(), 16);
        assert!(frames.iter().all(|f| pointers.contains(&f.cpus.as_ptr())));
        assert_eq!(state.bookkeeping_bytes(), bytes);
    }

    #[test]
    fn confirmed_exit_keeps_final_work_then_becomes_known_zero_even_on_unobserved_cpus() {
        let mut state = SharedAccounting::new(2);
        state.set_processes(&[101]);
        state.confirm_alive(101, 100);
        state.thread_start(11, 101, 100);
        state.switch(0, 0, 11, 100);
        let mut values = vec![None; 2];
        state.sample_into(101, &mut values).unwrap();
        state.advance(200);
        state.sample_into(101, &mut values).unwrap();
        assert_eq!(values, vec![Some(100.0), None]);
        state.confirm_alive(101, 210);
        state.process_end(101, 250);
        assert!(state.confirmed_exit(101));
        state.thread_end(11, 250);
        state.switch(0, 11, 0, 250);
        state.advance(300);
        state.sample_into(101, &mut values).unwrap();
        assert_eq!(
            values,
            vec![Some(50.0), None],
            "retain the final measured interval"
        );
        state.process_start(101, "reused.exe".into(), 350, false);
        state.thread_start(22, 101, 350);
        state.switch(0, 0, 22, 350);
        state.advance(400);
        state.sample_into(101, &mut values).unwrap();
        assert_eq!(
            values,
            vec![Some(0.0); 2],
            "a positively ended exact instance is idle on every CPU, and a reused PID must not inherit its slot"
        );
    }

    #[test]
    fn buffered_end_of_an_older_pid_cannot_retire_a_new_verified_instance() {
        let mut state = SharedAccounting::new(1);
        state.set_processes(&[101]);
        state.confirm_alive(101, 200);
        state.process_end(101, 150);
        assert!(
            !state.confirmed_exit(101),
            "this instance was proven alive after the buffered end event"
        );
    }

    #[test]
    fn contributors_include_unselected_and_exited_processes_without_pid_reuse() {
        let mut state = SharedAccounting::new(2);
        state.enable_contributors();
        state.process_start(101, "first.exe".into(), 100, false);
        state.process_start(202, "other.exe".into(), 100, false);
        state.thread_start(11, 101, 100);
        state.thread_start(22, 202, 100);
        state.switch(0, 0, 11, 100);
        state.switch(1, 0, 22, 100);
        let mut rows = Vec::new();
        state.sample_contributors(&mut rows).unwrap();
        state.process_end(101, 150);
        state.thread_end(11, 150);
        state.switch(0, 11, 0, 150);
        state.process_start(101, "reused.exe".into(), 150, false);
        state.thread_start(33, 101, 150);
        state.switch(0, 0, 33, 150);
        state.advance(200);
        state.sample_contributors(&mut rows).unwrap();
        let first = rows
            .iter()
            .find(|r| r.process.name.as_ref() == "first.exe")
            .unwrap();
        let reused = rows
            .iter()
            .find(|r| r.process.name.as_ref() == "reused.exe")
            .unwrap();
        assert_eq!(first.cpus, [Some(50.0), Some(0.0)]);
        assert_eq!(reused.cpus, [Some(50.0), Some(0.0)]);
        assert_ne!(first.process.instance, reused.process.instance);
        assert_eq!(
            rows.iter().find(|r| r.process.pid == 202).unwrap().cpus,
            [Some(0.0), Some(100.0)]
        );
        state.advance(300);
        state.sample_contributors(&mut rows).unwrap();
        assert!(!rows.iter().any(|r| r.process.name.as_ref() == "first.exe"));
        state.fail("lost events");
        assert!(state.sample_contributors(&mut rows).is_err());
        assert!(rows.is_empty());
    }
    fn selected_accounting(cpus: usize) -> SharedAccounting {
        let mut state = SharedAccounting::new(cpus);
        state.set_processes(&[1]);
        state
    }

    #[test]
    fn shared_accounting_keeps_instances_separate_and_admits_existing_threads() {
        let mut state = SharedAccounting::new(2);
        state.thread_start(11, 101, 100);
        state.thread_start(22, 202, 100);
        state.switch(0, 0, 11, 100);
        state.switch(1, 0, 22, 100);
        state.set_processes(&[101]);
        let mut a = vec![None; 2];
        state.sample_into(101, &mut a).unwrap();
        state.advance(200);
        state.sample_into(101, &mut a).unwrap();
        assert_eq!(a, vec![Some(100.0), Some(0.0)]);
        state.set_processes(&[101, 202]);
        let mut b = vec![None; 2];
        state.sample_into(202, &mut b).unwrap();
        state.advance(300);
        state.sample_into(101, &mut a).unwrap();
        state.sample_into(202, &mut b).unwrap();
        assert_eq!(a, vec![Some(100.0), Some(0.0)]);
        assert_eq!(b, vec![Some(0.0), Some(100.0)]);
        state.thread_end(22, 350);
        state.thread_start(22, 303, 350);
        state.set_processes(&[101]);
        state.advance(400);
        state.sample_into(101, &mut a).unwrap();
        assert_eq!(a, vec![Some(100.0), Some(0.0)]);
    }

    #[test]
    fn reused_process_slot_starts_a_new_interval_and_loss_keeps_data_unknown() {
        let mut state = SharedAccounting::new(1);
        state.set_processes(&[101]);
        state.thread_start(11, 101, 100);
        state.switch(0, 0, 11, 100);
        let mut values = vec![None];
        state.sample_into(101, &mut values).unwrap();
        state.advance(200);
        state.sample_into(101, &mut values).unwrap();
        assert_eq!(values, vec![Some(100.0)]);
        state.remove_process(101);
        state.set_processes(&[101]);
        state.sample_into(101, &mut values).unwrap();
        assert_eq!(values, vec![None]);
        state.fail("lost events");
        assert!(state.sample_into(101, &mut values).is_err());
        assert_eq!(values, vec![None]);
    }

    #[test]
    fn real_windows_cswitch_v5_uses_confirmed_prefix_without_tdh_metadata() {
        // Actual Windows 11 event: ThreadGuid / CSwitch opcode36 / v5 /28B.
        // IDs below are synthetic; only this observed shape is characterized.
        let mut payload = [0u8; 28];
        payload[..4].copy_from_slice(&11u32.to_le_bytes());
        payload[4..8].copy_from_slice(&22u32.to_le_bytes());
        assert_eq!(scheduler_pair(36, 5, &payload), Some(Ok((11, 22))));
        assert_eq!(scheduler_pair(36, 2, &payload[..24]), Some(Ok((11, 22))));
        for length in [0, 7, 8, 23, 24, 27] {
            assert!(scheduler_pair(36, 5, &payload[..length]).unwrap().is_err());
        }
        assert!(scheduler_pair(36, 2, &payload[..23]).unwrap().is_err());
        assert!(
            scheduler_pair(36, 6, &payload).is_none(),
            "unknown version still requires TDH"
        );
    }

    #[test]
    fn source_confirmed_thread_v3_keeps_ids_and_rejects_missing_prefix() {
        // Microsoft PerfView ThreadTraceData uses these same initial UInt32
        // fields for v>=1; v3 appends priorities/flags/name after the v2 body.
        let payload = [11, 0, 0, 0, 22, 0, 0, 0];
        for opcode in 1..=4 {
            assert_eq!(scheduler_pair(opcode, 3, &payload), Some(Ok((11, 22))));
            assert!(scheduler_pair(opcode, 3, &payload[..7]).unwrap().is_err());
        }
        assert!(scheduler_pair(3, 5, &payload).is_none());
        assert!(scheduler_pair(37, 3, &payload).is_none());
    }

    #[test]
    fn processor_groups_flatten_without_collisions_or_total_instances() {
        assert_eq!(
            processor_coordinates(&[2, 3]).unwrap(),
            vec![(0, 0), (0, 1), (1, 0), (1, 1), (1, 2)]
        );
        assert!(processor_coordinates(&[]).is_err());
        assert!(processor_coordinates(&[2, 0]).is_err());
    }

    #[test]
    fn process_percent_uses_whole_machine_and_rejects_counter_regression() {
        assert_eq!(process_percent(10, 5_000_010, 1_000_000_000, 4), Some(12.5));
        assert_eq!(process_percent(20, 10, 1_000_000_000, 4), None);
        assert_eq!(process_percent(0, 10, 0, 4), None);
        assert_eq!(process_percent(0, 10, 1, 0), None);
    }

    #[test]
    fn cswitch_accounts_only_observed_runtime_and_never_affinity() {
        let mut state = selected_accounting(2);
        state.thread_start(11, 1, 100);
        state.switch(0, 0, 11, 100);
        state.switch(1, 0, 99, 100);
        assert_eq!(state.sample().unwrap(), vec![None, None]);
        state.switch(0, 11, 99, 150);
        state.switch(1, 99, 11, 150);
        state.advance(200);
        assert_eq!(state.sample().unwrap(), vec![Some(50.0), Some(50.0)]);
        state.advance(300);
        assert_eq!(state.sample().unwrap(), vec![Some(0.0), Some(100.0)]);
    }

    #[test]
    fn end_and_reused_thread_id_do_not_transfer_target_ownership() {
        let mut state = selected_accounting(1);
        state.thread_start(11, 1, 100);
        state.switch(0, 0, 11, 100);
        state.sample().unwrap();
        state.thread_end(11, 140);
        state.thread_start(11, 2, 150);
        state.switch(0, 11, 11, 160);
        state.advance(200);
        assert_eq!(state.sample().unwrap(), vec![Some(40.0)]);
    }

    #[test]
    fn unobserved_cpu_is_missing_and_same_watermark_does_not_fabricate_zero() {
        let mut state = selected_accounting(2);
        state.switch(0, 0, 99, 100);
        state.sample().unwrap();
        assert_eq!(state.sample().unwrap(), vec![None, None]);
        state.advance(200);
        assert_eq!(state.sample().unwrap(), vec![Some(0.0), None]);
    }

    #[test]
    fn loss_unknown_cpu_and_out_of_order_events_are_terminal() {
        for fault in 0..4 {
            let mut state = selected_accounting(1);
            state.switch(0, 0, 99, 100);
            state.sample().unwrap();
            match fault {
                0 => state.fail("ETW lost events"),
                1 => state.switch(1, 0, 99, 110),
                2 => state.advance(90),
                _ => state.switch(0, 42, 99, 110),
            }
            assert!(state.sample().is_err());
            state.advance(200);
            assert!(state.sample().is_err());
        }
    }

    #[test]
    fn malformed_payload_and_unavailable_processor_index_do_not_get_guessed() {
        assert_eq!(two_u32(&[11, 0, 0, 0, 22, 0, 0, 0]), Some((11, 22)));
        assert_eq!(two_u32(&[0; 7]), None);
        assert_eq!(event_cpu_index(true, 300, 44, 400), Some(300));
        assert_eq!(event_cpu_index(false, 300, 44, 400), None);
        assert_eq!(event_cpu_index(false, 0, 44, 64), Some(44));
        assert_eq!(event_cpu_index(true, 400, 0, 400), None);
    }
}
