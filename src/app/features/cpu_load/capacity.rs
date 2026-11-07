//! Selected-group capacity, derived from the existing exact-instance ETW cut.
use super::{history::SampleTime, model::RuleTarget, process_list::CpuTotal};
use crate::app::shared::ids::GroupId;
use os_api::cpu_load::ProcessCpuSample;
use std::collections::HashMap;

pub(super) fn normalized_mask(mask: &[usize]) -> Vec<usize> {
    let mut mask = mask.to_vec();
    mask.sort_unstable();
    mask.dedup();
    mask
}

#[derive(Default)]
pub(super) struct CapacityValue {
    pub value: Option<f32>,
    pub complete: bool,
    pub text: String,
}

#[derive(Default)]
pub(super) struct CapacityTotals {
    pub masks: HashMap<GroupId, Vec<usize>>,
    pub rules: HashMap<RuleTarget, CpuTotal>,
    pub groups: HashMap<GroupId, CpuTotal>,
    pub values: HashMap<(GroupId, u32, u64), CapacityValue>,
    pub time: Option<SampleTime>,
}
impl CapacityTotals {
    pub fn build<'a>(
        rows: impl Iterator<Item = (&'a RuleTarget, u32, u64)>,
        masks: &HashMap<GroupId, Vec<usize>>,
        samples: &[ProcessCpuSample],
        time: Option<SampleTime>,
        cpu_count: usize,
    ) -> Self {
        let mut out = Self {
            masks: masks
                .iter()
                .map(|(g, m)| (g.clone(), normalized_mask(m)))
                .collect(),
            time,
            ..Default::default()
        };
        let valid = |mask: &[usize]| !mask.is_empty() && mask.iter().all(|&cpu| cpu < cpu_count);
        for (group, mask) in &out.masks {
            if time.is_some() && valid(mask) {
                out.groups.insert(
                    group.clone(),
                    CpuTotal {
                        value: Some(0.0),
                        ..Default::default()
                    },
                );
            }
        }
        for (target, pid, token) in rows {
            let key = (target.group.clone(), pid, token);
            let fresh = !out.values.contains_key(&key);
            let load = out.values.entry(key).or_insert_with(|| {
                let mask = out.masks.get(&target.group);
                let sample = samples
                    .binary_search_by_key(&(pid, token), |s| (s.pid, s.token))
                    .ok()
                    .and_then(|i| samples.get(i));
                let (mut sum, mut known) = (0.0, 0);
                let eligible = time.is_some()
                    && mask.is_some_and(|m| valid(m))
                    && sample.is_some_and(|s| s.cpus.len() == cpu_count);
                if eligible {
                    for &cpu in mask.unwrap() {
                        if let Some(value) = sample.unwrap().cpus[cpu]
                            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                        {
                            sum += value;
                            known += 1;
                        }
                    }
                }
                let value =
                    (known > 0).then(|| (sum / mask.unwrap().len() as f32).clamp(0.0, 100.0));
                let complete = eligible && known == mask.unwrap().len();
                CapacityValue {
                    value,
                    complete,
                    text: super::view::measured_percent(value, !complete),
                }
            });
            record(out.rules.entry(target.clone()).or_default(), load);
            if fresh {
                record(out.groups.entry(target.group.clone()).or_default(), load);
            }
        }
        out
    }
    pub fn matches(&self, group: &GroupId, mask: &[usize]) -> bool {
        self.masks
            .get(group)
            .is_some_and(|old| old.as_slice() == mask || *old == normalized_mask(mask))
    }
    pub fn reserved_bytes(&self) -> usize {
        self.masks.capacity() * std::mem::size_of::<(GroupId, Vec<usize>)>()
            + self
                .masks
                .iter()
                .map(|(g, m)| g.0.capacity() + m.capacity() * std::mem::size_of::<usize>())
                .sum::<usize>()
            + self.rules.capacity() * std::mem::size_of::<(RuleTarget, CpuTotal)>()
            + self
                .rules
                .keys()
                .map(|t| t.group.0.capacity() + t.rule.0.capacity())
                .sum::<usize>()
            + self.groups.capacity() * std::mem::size_of::<(GroupId, CpuTotal)>()
            + self.groups.keys().map(|g| g.0.capacity()).sum::<usize>()
            + self.values.capacity() * std::mem::size_of::<((GroupId, u32, u64), CapacityValue)>()
            + self
                .values
                .iter()
                .map(|((g, _, _), v)| g.0.capacity() + v.text.capacity())
                .sum::<usize>()
    }
}
fn record(total: &mut CpuTotal, load: &CapacityValue) {
    if total.instances == 0 {
        total.value = None;
    }
    total.instances += 1;
    total.measured += usize::from(load.complete);
    if let Some(value) = load.value {
        total.value = Some((total.value.unwrap_or(0.0) + value).min(100.0));
    }
}
