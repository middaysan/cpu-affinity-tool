use super::contributors::Contribution;
use super::history::{GapReason, HistoryStore, SampleTime};
use super::model::{RuleTarget, Snapshot};
use crate::app::shared::ids::RuleId;
use os_api::cpu_load::{CpuHistoryFrame, CpuHistoryTick, CpuProcess, ProcessCpuSample};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

const PENDING: usize = 16;
#[derive(Default)]
struct Ownership {
    generation: Option<u64>,
    named_count: usize,
    targets: HashMap<RuleTarget, Vec<(u32, u64)>>,
    runs: HashMap<String, (u32, u64)>,
    names: HashMap<(u32, u64), Arc<CpuProcess>>,
    rules: HashMap<RuleTarget, Arc<RuleId>>,
}

#[cfg(test)]
mod tests {
    use super::super::contributors::ContributorView;
    use super::super::history::HistoricalSample;
    use super::*;
    use crate::app::shared::ids::GroupId;
    use os_api::cpu_load::{CpuHistoryTarget, CpuHistoryValue};
    use std::time::{Duration, SystemTime};

    #[test]
    fn unordered_native_targets_keep_their_exact_loads_for_binary_search_consumers() {
        let target = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("r".into()),
        };
        let targets = HashMap::from([(target.clone(), vec![(10, 100), (20, 200)])]);
        let names = HashMap::new();
        let rules = HashMap::from([(target.clone(), Arc::new(target.rule.clone()))]);
        let runs = HashMap::new();
        let mut store = HistoryStore::default();
        store
            .configure(2, 30, std::slice::from_ref(&target))
            .unwrap();
        let mut events = EventHistory::default();
        for id in 0..=2 {
            events
                .enqueue(
                    CpuHistoryTick {
                        id,
                        seconds: id as f64 + 1.0,
                        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(id),
                    },
                    OwnerInput {
                        generation: Some(1),
                        targets: Some(&targets),
                        names: &names,
                        rules: &rules,
                        runs: &runs,
                        changed: false,
                    },
                    &[],
                    &mut store,
                )
                .unwrap();
        }
        let mut frame = CpuHistoryFrame::new(2).unwrap();
        frame.id = 1;
        frame.available = true;
        frame.cpus = vec![Some(20.0), Some(40.0)];
        frame.targets = vec![
            CpuHistoryTarget {
                pid: 20,
                token: 200,
            },
            CpuHistoryTarget {
                pid: 10,
                token: 100,
            },
        ];
        frame.coverage = vec![3, 3];
        frame.tracked = vec![
            CpuHistoryValue {
                target: 0,
                cpu: 1,
                load: 40.0,
            },
            CpuHistoryValue {
                target: 1,
                cpu: 0,
                load: 20.0,
            },
        ];
        events.apply(&frame, &mut store);
        assert_eq!(
            events
                .samples()
                .iter()
                .map(|s| (s.pid, s.token))
                .collect::<Vec<_>>(),
            [(10, 100), (20, 200)]
        );
        assert_eq!(
            events.samples()[0].cpus,
            [Some(20.0), Some(0.0)],
            "sorting must follow indexed native load assignment"
        );
        assert_eq!(
            events.snapshot.process_cpus[&target],
            [Some(20.0), Some(40.0)]
        );
    }

    #[test]
    fn delayed_run_work_uses_original_run_and_does_not_enter_saved_tracked_union() {
        let mut store = HistoryStore::default();
        store.configure(1, 30, &[]).unwrap();
        store
            .configure_runs(&["run-a".into(), "run-b".into()])
            .unwrap();
        let mut events = EventHistory::default();
        let names = HashMap::from([(
            (42, 10),
            Arc::new(CpuProcess {
                pid: 42,
                instance: 10,
                name: "child.exe".into(),
            }),
        )]);
        let rules = HashMap::new();
        let targets = HashMap::new();
        let old = HashMap::from([("run-a".to_string(), (42, 10))]);
        let new = HashMap::from([("run-b".to_string(), (42, 20))]);
        for id in 0..=2 {
            events
                .enqueue(
                    CpuHistoryTick {
                        id,
                        seconds: id as f64 + 1.0,
                        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(id),
                    },
                    OwnerInput {
                        generation: Some(1),
                        targets: Some(&targets),
                        names: &names,
                        rules: &rules,
                        runs: if id == 0 { &old } else { &new },
                        changed: true,
                    },
                    &[],
                    &mut store,
                )
                .unwrap();
        }
        let mut frame = CpuHistoryFrame::new(1).unwrap();
        frame.id = 0;
        events.apply(&frame, &mut store);
        frame.id = 1;
        frame.available = true;
        frame.cpus[0] = Some(25.0);
        frame.targets.push(CpuHistoryTarget { pid: 42, token: 10 });
        frame.coverage.push(1);
        frame.tracked.push(CpuHistoryValue {
            target: 0,
            cpu: 0,
            load: 25.0,
        });
        events.apply(&frame, &mut store);
        let mut sample = HistoricalSample::default();
        store.copy_sample(
            super::super::history::LoadSource::Run("run-a"),
            2.0,
            &mut sample,
        );
        assert_eq!(sample.values, [Some(25.0)]);
        store.copy_sample(
            super::super::history::LoadSource::Run("run-b"),
            2.0,
            &mut sample,
        );
        assert_eq!(sample.values, [None]);
        store.copy_sample(super::super::history::LoadSource::Tracked, 2.0, &mut sample);
        assert_eq!(sample.values, [Some(0.0)]);
        let mut view = ContributorView::default();
        store.contributors.copy(
            SampleTime {
                seconds: 2.0,
                wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
            },
            0,
            super::super::contributors::ContributorScope::Run("run-a"),
            &mut view,
        );
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].load, 25.0);
        store.configure(1, 300, &[]).unwrap();
        store.copy_sample(
            super::super::history::LoadSource::Run("run-a"),
            2.0,
            &mut sample,
        );
        assert_eq!(
            sample.values,
            [Some(25.0)],
            "retention resize preserves admitted run data"
        );
    }

    #[test]
    fn delayed_frames_keep_original_rule_and_creation_token_after_membership_changes() {
        let old = RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId("old".into()),
        };
        let new = RuleTarget {
            rule: RuleId("new".into()),
            ..old.clone()
        };
        let mut store = HistoryStore::default();
        store.configure(1, 30, &[old.clone(), new.clone()]).unwrap();
        let mut events = EventHistory::default();
        let names = HashMap::new();
        let rules = HashMap::from([
            (old.clone(), Arc::new(old.rule.clone())),
            (new.clone(), Arc::new(new.rule.clone())),
        ]);
        let old_targets = HashMap::from([(old.clone(), vec![(42, 10)])]);
        let new_targets = HashMap::from([(new.clone(), vec![(42, 20)])]);
        let tick = |id| CpuHistoryTick {
            id,
            seconds: id as f64 + 1.0,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(id),
        };
        for id in 0..=2 {
            events
                .enqueue(
                    tick(id),
                    OwnerInput {
                        generation: Some(id),
                        targets: Some(if id == 0 { &old_targets } else { &new_targets }),
                        names: &names,
                        rules: &rules,
                        runs: &HashMap::new(),
                        changed: false,
                    },
                    &[],
                    &mut store,
                )
                .unwrap();
        }
        let mut frame = CpuHistoryFrame::new(1).unwrap();
        frame.id = 0;
        events.apply(&frame, &mut store);
        frame.id = 1;
        frame.available = true;
        frame.cpus[0] = Some(25.0);
        frame.targets.push(CpuHistoryTarget { pid: 42, token: 10 });
        frame.coverage.push(1);
        frame.tracked.push(CpuHistoryValue {
            target: 0,
            cpu: 0,
            load: 25.0,
        });
        events.apply(&frame, &mut store);
        let mut sample = HistoricalSample::default();
        store.copy_sample(Some(&old), 2.0, &mut sample);
        assert_eq!(sample.values, [Some(25.0)]);
        store.copy_sample(Some(&new), 2.0, &mut sample);
        assert_eq!(sample.values, [None]);
        let time = SampleTime {
            seconds: 2.0,
            wall: tick(1).timestamp,
        };
        assert_eq!(events.capacity_time(time.seconds), Some(time));
        assert_eq!(
            events.capacity_time(time.seconds + 4.0),
            None,
            "waiting after a tick cannot refresh the original scheduler cut"
        );
        let masks = HashMap::from([(old.group.clone(), vec![0])]);
        let mut current = super::super::process_list::ProcessList::from_tracking(
            &new_targets,
            &HashMap::new(),
            &names,
        );
        current.record_cores(1, events.samples());
        current.record_capacity(&masks, events.samples(), events.capacity_time(time.seconds));
        events.qualify_capacity(&mut current, &new_targets);
        assert_eq!(
            current.rule_capacity_total(&new, &[0]).value,
            None,
            "a new owner/lifetime cannot reuse an old rule's ETW cut"
        );
        let empty = HashMap::new();
        let mut removed =
            super::super::process_list::ProcessList::from_tracking(&empty, &HashMap::new(), &names);
        removed.record_cores(1, events.samples());
        removed.record_capacity(&masks, events.samples(), events.capacity_time(time.seconds));
        events.qualify_capacity(&mut removed, &empty);
        assert_eq!(
            removed.group_capacity_total(&old.group, &[0]).value,
            None,
            "removing all claims changes the group union even without a remaining rule to reject"
        );
        let mut view = ContributorView::default();
        store.contributors.copy(time, 0, Some(&old.rule), &mut view);
        assert_eq!(view.rows[0].process.instance, 10);
        assert_eq!(view.rows[0].process.name.as_ref(), "Name unavailable");
        store.contributors.copy(time, 0, Some(&new.rule), &mut view);
        assert!(
            view.rows.is_empty(),
            "late old-token work must not be attributed to the current owner"
        );
        frame.id = 2;
        frame.targets[0].token = 20;
        frame.tracked[0].load = 75.0;
        events.apply(&frame, &mut store);
        store.copy_sample(Some(&new), 3.0, &mut sample);
        assert_eq!(sample.values, [Some(75.0)]);
        for id in 3..=6 {
            events
                .enqueue(
                    tick(id),
                    OwnerInput {
                        generation: Some(1),
                        targets: Some(&new_targets),
                        names: &names,
                        rules: &rules,
                        runs: &HashMap::new(),
                        changed: false,
                    },
                    &[],
                    &mut store,
                )
                .unwrap();
        }
        assert_eq!(
            events.capacity_time(tick(6).seconds),
            None,
            "fresh worker publications cannot refresh a four-second-old scheduler cut"
        );
        assert!(
            !events.samples().is_empty(),
            "history samples remain retained independently of current-meter freshness"
        );
        frame.id = 6;
        events.apply(&frame, &mut store);
        assert_eq!(
            events.capacity_time(tick(6).seconds).map(|t| t.seconds),
            Some(7.0),
            "a newly completed cut restores the capacity meter"
        );
        events.fail(&mut store);
        assert!(events.samples().is_empty());
        assert_eq!(events.capacity_time(tick(6).seconds), None);
    }
}
pub(super) struct OwnerInput<'a> {
    pub generation: Option<u64>,
    pub targets: Option<&'a HashMap<RuleTarget, Vec<(u32, u64)>>>,
    pub names: &'a HashMap<(u32, u64), Arc<CpuProcess>>,
    pub rules: &'a HashMap<RuleTarget, Arc<RuleId>>,
    pub runs: &'a HashMap<String, (u32, u64)>,
    pub changed: bool,
}
struct PendingInterval {
    id: u64,
    time: SampleTime,
    owners: Arc<Ownership>,
}
pub(super) struct EventHistory {
    pending: VecDeque<PendingInterval>,
    captured: Arc<Ownership>,
    current: Arc<Ownership>,
    samples: Vec<ProcessCpuSample>,
    sample_time: Option<SampleTime>,
    sample_owners: Arc<Ownership>,
    sample_count: usize,
    snapshot: Snapshot,
    inputs: Vec<Contribution>,
    unique: Vec<(u32, u64)>,
}
impl Default for EventHistory {
    fn default() -> Self {
        Self {
            pending: VecDeque::with_capacity(PENDING),
            captured: Arc::default(),
            current: Arc::default(),
            samples: Vec::new(),
            sample_time: None,
            sample_owners: Arc::default(),
            sample_count: 0,
            snapshot: Snapshot::default(),
            inputs: Vec::new(),
            unique: Vec::new(),
        }
    }
}
impl EventHistory {
    pub fn enqueue(
        &mut self,
        tick: CpuHistoryTick,
        input: OwnerInput<'_>,
        parking: &[Option<bool>],
        store: &mut HistoryStore,
    ) -> Result<(), String> {
        if self.pending.len() == PENDING {
            return Err("CPU history exceeded 16 pending intervals".into());
        }
        if input.changed
            || input.runs != &self.captured.runs
            || input.generation != self.captured.generation
            || input.names.len() != self.captured.named_count
        {
            let targets = input.targets.cloned().unwrap_or_default();
            let mut names = input.names.clone();
            for &(pid, token) in targets.values().flatten().chain(input.runs.values()) {
                names.entry((pid, token)).or_insert_with(|| {
                    Arc::new(CpuProcess {
                        pid,
                        instance: token,
                        name: "Name unavailable".into(),
                    })
                });
            }
            self.captured = Arc::new(Ownership {
                generation: input.generation,
                named_count: input.names.len(),
                targets,
                runs: input.runs.clone(),
                names,
                rules: input.rules.clone(),
            });
        }
        let time = SampleTime {
            seconds: tick.seconds,
            wall: tick.timestamp,
        };
        store.push_pending(time, parking);
        self.pending.push_back(PendingInterval {
            id: tick.id,
            time,
            owners: self.current.clone(),
        });
        self.current = self.captured.clone();
        Ok(())
    }
    #[cfg(test)]
    pub fn fail(&mut self, store: &mut HistoryStore) {
        self.fail_with_reason(store, GapReason::TraceUnavailable);
    }
    pub fn fail_with_reason(&mut self, store: &mut HistoryStore, reason: GapReason) {
        while let Some(pending) = self.pending.pop_front() {
            store.mark_lost_reason(pending.time, reason);
            store.contributors.push(pending.time, &[], false);
        }
        self.current = Arc::default();
        self.clear_values();
    }
    fn clear_values(&mut self) {
        self.sample_count = 0;
        self.sample_time = None;
        self.sample_owners = Arc::default();
        self.snapshot.tracked_cpus.fill(None);
        for values in self
            .snapshot
            .run_cpus
            .values_mut()
            .chain(self.snapshot.run_partial_cpus.values_mut())
        {
            values.fill(None);
        }
        self.snapshot.tracked_partial_cpus.fill(None);
        for values in self
            .snapshot
            .process_cpus
            .values_mut()
            .chain(self.snapshot.process_partial_cpus.values_mut())
        {
            values.fill(None);
        }
    }
    pub fn apply(&mut self, frame: &CpuHistoryFrame, store: &mut HistoryStore) {
        while self.pending.front().is_some_and(|p| p.id < frame.id) {
            let missing = self.pending.pop_front().unwrap();
            store.mark_lost_reason(missing.time, GapReason::MissingFrame);
            store.contributors.push(missing.time, &[], false);
        }
        let Some(pending) = self.pending.front().filter(|p| p.id == frame.id) else {
            return;
        };
        let time = pending.time;
        store.api_native_interval(time, frame.id, frame.begin, frame.end);
        let owners = pending.owners.clone();
        self.pending.pop_front();
        if !frame.available {
            self.clear_values();
            store.mark_lost_reason(time, frame.gap.unwrap_or(GapReason::Other));
            store.contributors.push(time, &[], false);
            return;
        }
        let cpus = frame.cpus.len();
        self.sample_time = Some(time);
        self.sample_owners = owners.clone();
        while self.samples.len() < frame.targets.len() {
            self.samples.push(ProcessCpuSample {
                pid: 0,
                token: 0,
                cpus: vec![None; cpus],
            });
        }
        self.sample_count = frame.targets.len();
        for (index, target) in frame.targets.iter().enumerate() {
            let sample = &mut self.samples[index];
            sample.pid = target.pid;
            sample.token = target.token;
            sample.cpus.resize(cpus, None);
            for (cpu, value) in sample.cpus.iter_mut().enumerate() {
                *value = frame.known(index, cpu).then_some(0.0);
            }
        }
        for value in &frame.tracked {
            if let Some(sample) = self
                .samples
                .get_mut(value.target as usize)
                .filter(|_| (value.target as usize) < self.sample_count)
            {
                if let Some(cell) = sample
                    .cpus
                    .get_mut(value.cpu as usize)
                    .filter(|v| v.is_some())
                {
                    *cell = Some(value.load);
                }
            }
        }
        // Native target indices are assigned before sorting for exact-instance lookups.
        self.samples[..self.sample_count].sort_unstable_by_key(|s| (s.pid, s.token));
        let samples = &self.samples[..self.sample_count];
        self.snapshot.cpus.clone_from(&frame.cpus);
        self.snapshot.tracked_cpus.resize(cpus, None);
        self.snapshot.tracked_cpus.fill(None);
        for values in self
            .snapshot
            .run_cpus
            .values_mut()
            .chain(self.snapshot.run_partial_cpus.values_mut())
        {
            values.fill(None);
        }
        self.snapshot.tracked_partial_cpus.resize(cpus, None);
        self.snapshot.tracked_partial_cpus.fill(None);
        self.snapshot
            .process_cpus
            .retain(|target, _| owners.targets.contains_key(target));
        self.snapshot
            .process_partial_cpus
            .retain(|target, _| owners.targets.contains_key(target));
        self.inputs.clear();
        self.unique.clear();
        if owners.generation.is_some() {
            self.unique.extend(
                owners
                    .targets
                    .values()
                    .flat_map(|instances| instances.iter().copied()),
            );
            self.unique.sort_unstable();
            self.unique.dedup();
            if self.unique.is_empty() {
                self.snapshot.tracked_cpus.fill(Some(0.0));
            } else {
                super::runtime::aggregate_cpus(
                    &mut self.snapshot.tracked_cpus,
                    &mut self.snapshot.tracked_partial_cpus,
                    &self.unique,
                    samples,
                );
            }
            for (target, instances) in &owners.targets {
                let values = self
                    .snapshot
                    .process_cpus
                    .entry(target.clone())
                    .or_default();
                values.resize(cpus, None);
                let partial = self
                    .snapshot
                    .process_partial_cpus
                    .entry(target.clone())
                    .or_default();
                partial.resize(cpus, None);
                super::runtime::aggregate_cpus(values, partial, instances, samples);
                for &(pid, token) in instances {
                    if let (Some(identity), Some(rule), Ok(index)) = (
                        owners.names.get(&(pid, token)),
                        owners.rules.get(target),
                        samples.binary_search_by_key(&(pid, token), |s| (s.pid, s.token)),
                    ) {
                        for (cpu, load) in samples[index].cpus.iter().enumerate() {
                            if let Some(load) = load.filter(|v| *v > 0.0) {
                                self.inputs.push(Contribution {
                                    process: identity.clone(),
                                    rule: Some(rule.clone()),
                                    run: None,
                                    cpu,
                                    load,
                                });
                            }
                        }
                    }
                }
            }
        }
        self.snapshot
            .run_cpus
            .retain(|id, _| owners.runs.contains_key(id));
        self.snapshot
            .run_partial_cpus
            .retain(|id, _| owners.runs.contains_key(id));
        for (id, &instance) in &owners.runs {
            let values = self.snapshot.run_cpus.entry(id.clone()).or_default();
            values.resize(cpus, None);
            let partial = self
                .snapshot
                .run_partial_cpus
                .entry(id.clone())
                .or_default();
            partial.resize(cpus, None);
            super::runtime::aggregate_cpus(values, partial, &[instance], samples);
            if let (Some(identity), Ok(index)) = (
                owners.names.get(&instance),
                samples.binary_search_by_key(&instance, |s| (s.pid, s.token)),
            ) {
                for (cpu, load) in samples[index].cpus.iter().enumerate() {
                    if let Some(load) = load.filter(|v| *v > 0.0) {
                        self.inputs.push(Contribution {
                            process: identity.clone(),
                            rule: None,
                            run: Some(Arc::from(id.as_str())),
                            cpu,
                            load,
                        });
                    }
                }
            }
        }
        for value in &frame.contributors {
            self.inputs.push(Contribution {
                process: value.process.clone(),
                rule: None,
                run: None,
                cpu: value.cpu as usize,
                load: value.load,
            });
        }
        if let Some(row) = frame.contributors.first() {
            for (cpu, value) in frame.cpus.iter().enumerate() {
                if value.is_none() {
                    self.inputs.push(Contribution {
                        process: row.process.clone(),
                        rule: None,
                        run: None,
                        cpu,
                        load: f32::NAN,
                    });
                }
            }
        }
        if store.complete(time, &self.snapshot) {
            store.contributors.push(time, &self.inputs, true);
            if frame.partial {
                store.contributors.mark_system_partial();
            }
        }
    }
    pub fn publish(&self, snapshot: &mut Snapshot) {
        snapshot.run_cpus.clone_from(&self.snapshot.run_cpus);
        snapshot
            .run_partial_cpus
            .clone_from(&self.snapshot.run_partial_cpus);
        snapshot
            .process_cpus
            .clone_from(&self.snapshot.process_cpus);
        snapshot
            .process_partial_cpus
            .clone_from(&self.snapshot.process_partial_cpus);
        snapshot
            .tracked_cpus
            .clone_from(&self.snapshot.tracked_cpus);
        snapshot
            .tracked_partial_cpus
            .clone_from(&self.snapshot.tracked_partial_cpus);
    }
    pub fn samples(&self) -> &[ProcessCpuSample] {
        &self.samples[..self.sample_count]
    }
    pub fn capacity_time(&self, now: f64) -> Option<SampleTime> {
        let time = self.sample_time?;
        let age = now - time.seconds;
        (self.sample_owners.generation.is_some() && age.is_finite() && (0.0..4.0).contains(&age))
            .then_some(time)
    }
    pub fn qualify_capacity(
        &self,
        list: &mut super::process_list::ProcessList,
        targets: &HashMap<RuleTarget, Vec<(u32, u64)>>,
    ) {
        // Global roster changes cannot relabel an old interval. Only exact
        // unchanged rule memberships may reuse that completed interval.
        let unions = |targets: &HashMap<RuleTarget, Vec<(u32, u64)>>| {
            let mut groups = HashMap::<_, HashSet<_>>::new();
            for (target, members) in targets {
                for &member in members {
                    groups
                        .entry(target.group.clone())
                        .or_default()
                        .insert(member);
                }
            }
            groups
        };
        let previous_groups = unions(&self.sample_owners.targets);
        let current_groups = unions(targets);
        list.qualify_capacity(
            |target| {
                targets
                    .get(target)
                    .is_some_and(|members| self.sample_owners.targets.get(target) == Some(members))
            },
            |group| current_groups.get(group) == previous_groups.get(group),
        );
    }
    pub fn bookkeeping_bytes(&self) -> usize {
        // Shared owner snapshots are counted once, including old memberships
        // retained only while their original intervals await delivery.
        let mut seen = [std::ptr::null(); PENDING + 3];
        let mut count = 0;
        let mut owners_bytes = 0;
        for owners in std::iter::once(&self.captured)
            .chain(std::iter::once(&self.current))
            .chain(std::iter::once(&self.sample_owners))
            .chain(self.pending.iter().map(|p| &p.owners))
        {
            let pointer = Arc::as_ptr(owners);
            if seen[..count].contains(&pointer) {
                continue;
            }
            seen[count] = pointer;
            count += 1;
            owners_bytes += owners.runs.capacity() * std::mem::size_of::<(String, (u32, u64))>()
                + owners.runs.keys().map(String::capacity).sum::<usize>();
            owners_bytes += owners.targets.capacity()
                * std::mem::size_of::<(RuleTarget, Vec<(u32, u64)>)>()
                + owners.names.capacity() * std::mem::size_of::<((u32, u64), Arc<CpuProcess>)>()
                + owners.rules.capacity() * std::mem::size_of::<(RuleTarget, Arc<RuleId>)>()
                + owners
                    .targets
                    .values()
                    .map(|v| v.capacity() * std::mem::size_of::<(u32, u64)>())
                    .sum::<usize>();
        }
        owners_bytes
            + self.samples.capacity() * std::mem::size_of::<ProcessCpuSample>()
            + self
                .samples
                .iter()
                .map(|row| row.cpus.capacity() * std::mem::size_of::<Option<f32>>())
                .sum::<usize>()
            + self.inputs.capacity() * std::mem::size_of::<Contribution>()
            + self.unique.capacity() * std::mem::size_of::<(u32, u64)>()
            + self.pending.capacity() * std::mem::size_of::<PendingInterval>()
    }
}
