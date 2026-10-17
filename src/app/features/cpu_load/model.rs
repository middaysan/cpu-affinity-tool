use crate::app::shared::ids::{GroupId, RuleId};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RuleTarget {
    pub group: GroupId,
    pub rule: RuleId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GraphTarget {
    Group {
        id: GroupId,
        name: String,
        cpus: Vec<usize>,
    },
    Process {
        target: RuleTarget,
        name: String,
    },
}
#[derive(Clone)]
pub(super) struct Snapshot {
    pub revision: u64,
    pub at: Instant,
    pub cpus: Vec<Option<f32>>,
    pub tracked_cpus: Vec<Option<f32>>,
    pub tracked_partial_cpus: Vec<Option<f32>>,
    pub parked: Vec<Option<bool>>,
    pub processes: HashMap<RuleTarget, Option<f32>>,
    pub process_cpus: HashMap<RuleTarget, Vec<Option<f32>>>,
    pub process_partial_cpus: HashMap<RuleTarget, Vec<Option<f32>>>,
    pub live_processes: std::sync::Arc<super::process_list::ProcessList>,
    pub error: Option<String>,
    pub detail_error: Option<String>,
    pub history_bytes: usize,
    pub instances: usize,
    pub bookkeeping_bytes: usize,
    pub threads: usize,
    pub contributor_metadata_bytes: usize,
    pub contributor_identities: usize,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            at: Instant::now(),
            cpus: Vec::new(),
            tracked_cpus: Vec::new(),
            tracked_partial_cpus: Vec::new(),
            parked: Vec::new(),
            processes: HashMap::new(),
            process_cpus: HashMap::new(),
            process_partial_cpus: HashMap::new(),
            live_processes: Default::default(),
            error: None,
            detail_error: None,
            history_bytes: 0,
            instances: 0,
            bookkeeping_bytes: 0,
            threads: 0,
            contributor_metadata_bytes: 0,
            contributor_identities: 0,
        }
    }
}
/// CPU groups describe capacity, independently of application ownership.
pub(super) fn group_mean(cpus: &[Option<f32>], selected: &[usize]) -> Option<f32> {
    let mut count = 0;
    let mut total = 0.0;
    for (index, &cpu) in selected.iter().enumerate() {
        if selected[..index].contains(&cpu) {
            continue;
        }
        total += cpus.get(cpu).copied().flatten().filter(|v| v.is_finite())?;
        count += 1;
    }
    (count > 0).then(|| (total / count as f32).clamp(0.0, 100.0))
}
pub(super) fn normalized_instances(
    instances: impl IntoIterator<Item = (u32, u64)>,
) -> Vec<(u32, u64)> {
    let mut instances: Vec<_> = instances
        .into_iter()
        .filter(|(pid, token)| *pid != 0 && *token != 0)
        .collect();
    instances.sort_unstable();
    instances.dedup();
    instances
}
pub(super) fn process_totals(
    targets: &HashMap<RuleTarget, Vec<(u32, u64)>>,
    mut read: impl FnMut(u32, u64) -> Option<f32>,
) -> HashMap<RuleTarget, Option<f32>> {
    let mut values = HashMap::new();
    targets
        .iter()
        .map(|(target, instances)| {
            let total = instances.iter().try_fold(0.0, |sum, &(pid, token)| {
                let value = *values
                    .entry((pid, token))
                    .or_insert_with(|| read(pid, token));
                value.map(|value| sum + value)
            });
            (target.clone(), total.map(|v| v.clamp(0.0, 100.0)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::history::{HistoryStore, Series};
    use super::*;
    use std::time::SystemTime;
    fn target(name: &str) -> RuleTarget {
        RuleTarget {
            group: GroupId("g".into()),
            rule: RuleId(name.into()),
        }
    }
    #[test]
    fn continuous_history_is_reserved_before_sampling_and_survives_process_churn() {
        let a = target("a");
        let b = target("b");
        let mut store = HistoryStore::default();
        store.configure(2, 30, &[a.clone(), b.clone()]).unwrap();
        let bytes = store.reserved_bytes();
        assert!(bytes >= 31 * 2 * 5 * 3);
        let mut values = HashMap::from([
            (a.clone(), vec![Some(20.0), Some(30.0)]),
            (b.clone(), vec![Some(40.0), None]),
        ]);
        for second in 0..100 {
            if second == 98 {
                values.remove(&a);
            }
            store.push(
                second as f64,
                SystemTime::UNIX_EPOCH,
                &[Some(50.0); 2],
                &[Some(true), None],
                &values,
            );
            assert_eq!(store.reserved_bytes(), bytes);
        }
        let mut series = Series::default();
        store.copy_series(Some(&a), 0, &mut series);
        assert_eq!(series.points.len(), 31);
        assert_eq!(series.points[28].value, Some(20.0));
        assert_eq!(series.points[30].value, None);
        store.copy_series(Some(&b), 0, &mut series);
        assert_eq!(series.points[30].value, Some(40.0));
        store.configure(2, 300, std::slice::from_ref(&b)).unwrap();
        store.copy_series(Some(&b), 0, &mut series);
        assert_eq!(series.points.len(), 31);
        assert_eq!(series.points[30].parked, Some(true));
    }
    #[test]
    fn adding_and_removing_rules_reserves_and_releases_only_their_history() {
        let a = target("a");
        let b = target("b");
        let mut store = HistoryStore::default();
        let mut series = Series::default();
        store.configure(2, 30, std::slice::from_ref(&a)).unwrap();
        store.push(
            1.0,
            SystemTime::UNIX_EPOCH,
            &[Some(10.0); 2],
            &[],
            &HashMap::from([(a.clone(), vec![Some(5.0); 2])]),
        );
        let before = store.reserved_bytes();
        store.configure(2, 30, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(store.reserved_bytes() - before, 31 * 2 * 5);
        store.copy_series(Some(&b), 0, &mut series);
        assert!(
            series.points.iter().all(|p| p.value.is_none()),
            "no fabricated pre-addition measurements"
        );
        store.configure(2, 30, std::slice::from_ref(&a)).unwrap();
        assert_eq!(store.reserved_bytes(), before);
        store.copy_series(Some(&a), 0, &mut series);
        assert_eq!(series.points[0].value, Some(5.0));
        store.push(100.0, SystemTime::UNIX_EPOCH, &[], &[], &HashMap::new());
        store.copy_series(None, 0, &mut series);
        assert_eq!(series.points.len(), 1, "elapsed gaps evict old samples");
    }
    #[test]
    fn group_mean_and_exact_instance_totals_preserve_unknown_and_shared_semantics() {
        assert_eq!(
            group_mean(&[Some(20.0), Some(80.0)], &[0, 1, 1]),
            Some(50.0)
        );
        assert_eq!(group_mean(&[Some(20.0), None], &[0, 1]), None);
        assert_eq!(group_mean(&[], &[]), None);
        assert_eq!(
            normalized_instances([(1, 10), (1, 10), (0, 10), (1, 0)]),
            vec![(1, 10)]
        );
        let a = target("a");
        let b = target("b");
        let targets = HashMap::from([
            (a.clone(), vec![(1, 10)]),
            (b.clone(), vec![(1, 10), (2, 20)]),
        ]);
        let mut reads = 0;
        let result = process_totals(&targets, |pid, _| {
            reads += 1;
            (pid == 1).then_some(5.0)
        });
        assert_eq!(reads, 2);
        assert_eq!(result[&a], Some(5.0));
        assert_eq!(result[&b], None);
    }

    #[test]
    fn moving_a_rule_keeps_history_and_oversized_configuration_is_refused() {
        let a = target("a");
        let mut store = HistoryStore::default();
        let mut series = Series::default();
        store.configure(2, 30, std::slice::from_ref(&a)).unwrap();
        store.push(
            1.0,
            SystemTime::UNIX_EPOCH,
            &[],
            &[],
            &HashMap::from([(a.clone(), vec![Some(10.0); 2])]),
        );
        let moved = RuleTarget {
            group: GroupId("other".into()),
            rule: a.rule.clone(),
        };
        store
            .configure(2, 30, std::slice::from_ref(&moved))
            .unwrap();
        store.copy_series(Some(&moved), 0, &mut series);
        assert_eq!(series.points[0].value, Some(10.0));
        let many: Vec<_> = (0..100).map(|n| target(&format!("r{n}"))).collect();
        assert!(store.configure(1024, 1800, &many).is_err());
        store.copy_series(Some(&moved), 0, &mut series);
        assert_eq!(
            series.points[0].value,
            Some(10.0),
            "failed reservation cannot discard existing history"
        );
    }
}
