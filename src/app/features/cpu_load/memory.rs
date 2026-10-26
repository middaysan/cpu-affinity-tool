use super::model::RuleTarget;
use crate::app::shared::ids::GroupId;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MemoryUsage {
    pub bytes: Option<u64>,
    pub measured: usize,
    pub instances: usize,
}

impl MemoryUsage {
    pub fn zero() -> Self {
        Self {
            bytes: Some(0),
            ..Default::default()
        }
    }

    fn record(&mut self, value: Option<u64>) {
        self.instances += 1;
        if let Some(value) = value {
            self.measured += 1;
            self.bytes = self.bytes.and_then(|sum| sum.checked_add(value));
        }
    }

    pub fn label(self) -> String {
        let Some(bytes) = self.bytes else {
            return "—".into();
        };
        let mib = 1024_u64 * 1024;
        let mut label = if bytes >= mib * 1024 {
            format!("{:.1} GB", bytes as f64 / (mib * 1024) as f64)
        } else if bytes > 0 && bytes < mib {
            "<1 MB".into()
        } else {
            format!("{} MB", bytes / mib)
        };
        if self.measured < self.instances {
            label.push('*');
        }
        label
    }
}

pub(super) struct MemoryTotals {
    pub rules: HashMap<RuleTarget, MemoryUsage>,
    pub groups: HashMap<GroupId, MemoryUsage>,
}

pub(super) fn tracked_memory(
    targets: &HashMap<RuleTarget, Vec<(u32, u64)>>,
    mut read: impl FnMut(u32, u64) -> Option<u64>,
) -> MemoryTotals {
    let mut values = HashMap::new();
    let mut unique = HashSet::new();
    let mut group_instances = HashMap::<&GroupId, HashSet<(u32, u64)>>::new();
    let mut rules = HashMap::new();
    let mut groups = HashMap::new();
    for (target, instances) in targets {
        unique.clear();
        let mut usage = MemoryUsage::zero();
        for &(pid, token) in instances {
            if pid == 0 || token == 0 || !unique.insert((pid, token)) {
                continue;
            }
            let value = *values
                .entry((pid, token))
                .or_insert_with(|| read(pid, token));
            usage.record(value);
            if group_instances
                .entry(&target.group)
                .or_default()
                .insert((pid, token))
            {
                groups
                    .entry(target.group.clone())
                    .or_insert_with(MemoryUsage::zero)
                    .record(value);
            }
        }
        usage.bytes = usage.bytes.filter(|_| usage.measured > 0);
        rules.insert(target.clone(), usage);
    }
    for usage in groups.values_mut() {
        usage.bytes = usage.bytes.filter(|_| usage.measured > 0);
    }
    MemoryTotals { rules, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shared::ids::{GroupId, RuleId};
    const MIB: u64 = 1024 * 1024;
    fn target(id: &str) -> RuleTarget {
        RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId(id.into()),
        }
    }

    #[test]
    fn memory_counts_added_names_once_per_exact_instance_and_preserves_partial_sum() {
        let a = target("a");
        let b = target("b");
        let targets = HashMap::from([
            (a.clone(), vec![(1, 100), (2, 200), (2, 200)]),
            (b.clone(), vec![(2, 200), (3, 300)]),
        ]);
        let mut calls = HashMap::new();
        let result = tracked_memory(&targets, |pid, token| {
            *calls.entry((pid, token)).or_insert(0) += 1;
            match pid {
                1 => Some(100 * MIB),
                2 => Some(200 * MIB),
                _ => None,
            }
        });
        assert_eq!(
            result.rules.get(&a),
            Some(&MemoryUsage {
                bytes: Some(300 * MIB),
                measured: 2,
                instances: 2
            })
        );
        assert_eq!(
            result.rules.get(&b),
            Some(&MemoryUsage {
                bytes: Some(200 * MIB),
                measured: 1,
                instances: 2
            })
        );
        assert_eq!(calls.len(), 3);
        assert!(calls.values().all(|&n| n == 1));
        assert_eq!(result.rules[&a].label(), "300 MB");
        assert_eq!(result.rules[&b].label(), "200 MB*");
        assert_eq!(
            result.groups[&a.group],
            MemoryUsage {
                bytes: Some(300 * MIB),
                measured: 2,
                instances: 3
            }
        );
    }

    #[test]
    fn memory_churn_never_carries_old_pid_values_into_a_new_instance() {
        let rule = target("a");
        for (instances, expected) in [
            (vec![(10, 100)], Some(32 * MIB)),
            (vec![(10, 200)], None),
            (vec![], None),
            (vec![(20, 300)], Some(0)),
        ] {
            let result = tracked_memory(
                &HashMap::from([(rule.clone(), instances)]),
                |pid, token| match (pid, token) {
                    (10, 100) => Some(32 * MIB),
                    (20, 300) => Some(0),
                    _ => None,
                },
            );
            assert_eq!(result.rules.get(&rule).and_then(|v| v.bytes), expected);
            assert_eq!(
                result.groups.get(&rule.group).and_then(|v| v.bytes),
                expected
            );
        }
        let result = tracked_memory(
            &HashMap::from([(rule.clone(), vec![(1, 1), (2, 2)])]),
            |pid, _| Some(if pid == 1 { u64::MAX } else { 1 }),
        );
        assert_eq!(
            result.rules.get(&rule).and_then(|v| v.bytes),
            None,
            "overflow is unavailable, not wrapped or saturated RAM"
        );
        assert_eq!(result.groups[&rule.group].bytes, None);
    }

    #[test]
    fn groups_count_shared_processes_once_and_reuse_reads_across_groups() {
        let a = target("a");
        let b = target("b");
        let c = RuleTarget {
            group: GroupId("other".into()),
            rule: RuleId("c".into()),
        };
        let targets = HashMap::from([
            (a.clone(), vec![(1, 100), (2, 200), (2, 200), (0, 0)]),
            (b.clone(), vec![(2, 200), (3, 300)]),
            (c.clone(), vec![(2, 200), (4, 400)]),
        ]);
        let mut calls = HashMap::new();
        let result = tracked_memory(&targets, |pid, token| {
            *calls.entry((pid, token)).or_insert(0) += 1;
            match pid {
                1 => Some(64 * MIB),
                2 => Some(96 * MIB),
                4 => Some(32 * MIB),
                _ => None,
            }
        });
        assert_eq!(
            result.groups[&a.group],
            MemoryUsage {
                bytes: Some(160 * MIB),
                measured: 2,
                instances: 3
            }
        );
        assert_eq!(
            result.groups[&c.group],
            MemoryUsage {
                bytes: Some(128 * MIB),
                measured: 2,
                instances: 2
            }
        );
        assert_eq!(result.rules[&a].bytes, Some(160 * MIB));
        assert_eq!(result.rules[&b].bytes, Some(96 * MIB));
        assert_eq!(calls.len(), 4);
        assert!(
            calls.values().all(|&count| count == 1),
            "group totals must add no native memory queries"
        );
    }

    #[test]
    fn memory_labels_are_compact_and_distinguish_unknown_from_real_zero() {
        for (bytes, label) in [
            (None, "—"),
            (Some(0), "0 MB"),
            (Some(1), "<1 MB"),
            (Some(5 * MIB), "5 MB"),
            (Some(1536 * MIB), "1.5 GB"),
        ] {
            assert_eq!(
                MemoryUsage {
                    bytes,
                    measured: 1,
                    instances: 1
                }
                .label(),
                label
            );
        }
    }
}
