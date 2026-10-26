use super::model::RuleTarget;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MemoryUsage {
    pub bytes: Option<u64>,
    pub measured: usize,
    pub instances: usize,
}

impl MemoryUsage {
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

pub(super) fn rule_memory(
    targets: &HashMap<RuleTarget, Vec<(u32, u64)>>,
    mut read: impl FnMut(u32, u64) -> Option<u64>,
) -> HashMap<RuleTarget, MemoryUsage> {
    let mut values = HashMap::new();
    let mut unique = HashSet::new();
    targets
        .iter()
        .map(|(target, instances)| {
            unique.clear();
            let mut usage = MemoryUsage::default();
            let mut bytes = Some(0_u64);
            for &(pid, token) in instances {
                if pid == 0 || token == 0 || !unique.insert((pid, token)) {
                    continue;
                }
                usage.instances += 1;
                if let Some(value) = *values
                    .entry((pid, token))
                    .or_insert_with(|| read(pid, token))
                {
                    usage.measured += 1;
                    bytes = bytes.and_then(|sum| sum.checked_add(value));
                }
            }
            usage.bytes = bytes.filter(|_| usage.measured > 0);
            (target.clone(), usage)
        })
        .collect()
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
        let result = rule_memory(&targets, |pid, token| {
            *calls.entry((pid, token)).or_insert(0) += 1;
            match pid {
                1 => Some(100 * MIB),
                2 => Some(200 * MIB),
                _ => None,
            }
        });
        assert_eq!(
            result.get(&a),
            Some(&MemoryUsage {
                bytes: Some(300 * MIB),
                measured: 2,
                instances: 2
            })
        );
        assert_eq!(
            result.get(&b),
            Some(&MemoryUsage {
                bytes: Some(200 * MIB),
                measured: 1,
                instances: 2
            })
        );
        assert_eq!(calls.len(), 3);
        assert!(calls.values().all(|&n| n == 1));
        assert_eq!(result[&a].label(), "300 MB");
        assert_eq!(result[&b].label(), "200 MB*");
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
            let result =
                rule_memory(
                    &HashMap::from([(rule.clone(), instances)]),
                    |pid, token| match (pid, token) {
                        (10, 100) => Some(32 * MIB),
                        (20, 300) => Some(0),
                        _ => None,
                    },
                );
            assert_eq!(result.get(&rule).and_then(|v| v.bytes), expected);
        }
        let result = rule_memory(
            &HashMap::from([(rule.clone(), vec![(1, 1), (2, 2)])]),
            |pid, _| Some(if pid == 1 { u64::MAX } else { 1 }),
        );
        assert_eq!(
            result.get(&rule).and_then(|v| v.bytes),
            None,
            "overflow is unavailable, not wrapped or saturated RAM"
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
