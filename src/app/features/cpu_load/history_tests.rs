use super::*;

#[cfg(all(debug_assertions, feature = "debug-mcp"))]
#[test]
fn cpu19_gap_reason_survives_resize_and_is_cleared_when_row_is_reused() {
    let mut store = HistoryStore::default();
    store.configure(1, 30, &[]).unwrap();
    let time = |seconds| SampleTime {
        seconds,
        wall: SystemTime::UNIX_EPOCH,
    };
    store.push_pending(time(1.0), &[]);
    store.mark_lost_reason(time(1.0), GapReason::ContributorCapacity);
    store.configure(1, 300, &[]).unwrap();
    let q = store.debug_quality(LoadSource::System, Some(1.0), 0, 0, 10);
    assert_eq!(q["selected"]["gap_reason"], "contributor_capacity");
    assert_eq!(q["selected"]["reason"], "interval_lost");
    store.configure(1, 30, &[]).unwrap();
    for t in 2..=34 {
        store.push_pending(time(t as f64), &[]);
    }
    let q = store.debug_quality(LoadSource::System, Some(34.0), 0, 0, 1);
    assert!(q["selected"]["gap_reason"].is_null());
    assert!(store.debug_quality(LoadSource::System, Some(1.0), 0, 0, 1)["selected"].is_null());
}

#[cfg(all(debug_assertions, feature = "debug-mcp"))]
#[test]
fn debug_quality_distinguishes_delivery_partial_load_and_exact_missing_time() {
    let mut store = HistoryStore::default();
    store.configure(2, 30, &[]).unwrap();
    let time = |seconds: u64| SampleTime {
        seconds: seconds as f64,
        wall: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
    };
    store.push_pending(time(1), &[]);
    store.push_pending(time(2), &[]);
    store.complete(
        time(2),
        &super::super::model::Snapshot {
            cpus: vec![Some(50.0), None],
            tracked_partial_cpus: vec![Some(12.0), None],
            ..Default::default()
        },
    );
    store.push_pending(time(3), &[]);
    store.mark_lost(time(3));
    let value = store.debug_quality(LoadSource::System, Some(2.0), 0, 0, 10);
    assert_eq!(
        value["delivery_counts"],
        serde_json::json!({"pending":1,"completed":1,"lost":1})
    );
    assert_eq!(value["selected"]["seconds"], 2.0);
    assert_eq!(value["selected"]["load_quality"], "complete");
    assert_eq!(value["intervals"][0]["delivery"], "lost");
    assert_eq!(
        store.debug_quality(LoadSource::Tracked, Some(2.0), 0, 0, 10)["selected"]["load_quality"],
        "partial"
    );
    assert!(store.debug_quality(LoadSource::System, Some(2.1), 0, 0, 10)["selected"].is_null());
}

#[test]
fn native_probe_distinguishes_pending_complete_lost_and_startup() {
    let mut store = HistoryStore::default();
    store.configure(2, 30, &[]).unwrap();
    let time = |seconds| SampleTime {
        seconds,
        wall: SystemTime::UNIX_EPOCH,
    };
    store.push_pending(time(1.0), &[]);
    store.push_pending(time(2.0), &[]);
    store.complete(
        time(2.0),
        &super::super::model::Snapshot {
            cpus: vec![Some(5.0), Some(0.0)],
            ..Default::default()
        },
    );
    store.push_pending(time(3.0), &[]);
    store.mark_lost(time(3.0));
    store.push_pending(time(4.0), &[]);
    let mut rows = Vec::new();
    store.probe_rows(2.0, &mut rows);
    assert_eq!(
        rows,
        vec![(time(2.0), 1, 0), (time(3.0), 2, 2), (time(4.0), 0, 2)]
    );
}

#[test]
fn delayed_completion_updates_only_the_original_row_and_preserves_loss_gaps() {
    let t = target();
    let mut store = HistoryStore::default();
    store.configure(1, 30, std::slice::from_ref(&t)).unwrap();
    let first = SampleTime {
        seconds: 10.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    let later = SampleTime {
        seconds: 11.0,
        wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
    };
    store.push_pending(first, &[]);
    store.push_pending(later, &[]);
    assert_eq!(store.pending_count(), 2);
    assert!(store.latest_finalized().is_none());
    let snapshot = super::super::model::Snapshot {
        cpus: vec![Some(50.0)],
        tracked_cpus: vec![Some(50.0)],
        process_cpus: HashMap::from([(t.clone(), vec![Some(50.0)])]),
        ..Default::default()
    };
    let before = store.reserved_bytes();
    assert!(store.complete(first, &snapshot));
    assert_eq!(store.pending_count(), 1);
    assert_eq!(store.latest_finalized(), Some(first));
    assert_eq!(store.value_at(Some(&t), first, 0), Some(50.0));
    assert!(store.value_at(Some(&t), later, 0).is_none());
    store.mark_lost(later);
    assert_eq!(store.pending_count(), 0);
    assert_eq!(store.latest_finalized(), Some(later));
    store.configure(1, 300, std::slice::from_ref(&t)).unwrap();
    assert_eq!(
        store.latest_finalized(),
        Some(later),
        "retention changes keep a resolved loss visible to Live"
    );
    store.configure(1, 30, std::slice::from_ref(&t)).unwrap();
    assert!(
        !store.complete(later, &snapshot),
        "a real-loss interval cannot later be relabeled as complete"
    );
    assert_eq!(store.reserved_bytes(), before);
    store.push_pending(
        SampleTime {
            seconds: 100.0,
            ..later
        },
        &[],
    );
    assert!(
        !store.complete(first, &snapshot),
        "evicted points cannot spill into a current slot"
    );
}

#[test]
fn partial_work_retains_quality_and_reserved_storage_through_history_resize() {
    let t = target();
    let mut store = HistoryStore::default();
    store.configure(2, 30, std::slice::from_ref(&t)).unwrap();
    let before = store.reserved_bytes();
    store.push(
        1.0,
        SystemTime::UNIX_EPOCH,
        &[Some(80.0); 2],
        &[],
        &HashMap::new(),
    );
    store.record_partial(
        &[Some(1.25), None],
        &HashMap::from([(t.clone(), vec![Some(1.25), None])]),
    );
    assert_eq!(
        store.reserved_bytes(),
        before,
        "quality belongs in the existing validity bytes"
    );
    let mut series = Series::default();
    store.copy_series(LoadSource::Tracked, 0, &mut series);
    assert_eq!(series.points[0].value, Some(1.25));
    assert!(series.points[0].partial);
    let mut sample = HistoricalSample::default();
    store.copy_sample(Some(&t), 1.0, &mut sample);
    assert_eq!(sample.values, vec![None; 2]);
    assert_eq!(sample.partial, vec![Some(1.25), None]);
    let mut overview = Overview::default();
    store.copy_overview(Some(&t), &mut overview);
    assert_eq!(overview.values[0], Some((1.25, 1.25)));
    assert!(overview.partial[0] && overview.activity[0]);
    store.configure(2, 300, std::slice::from_ref(&t)).unwrap();
    store.copy_series(Some(&t), 0, &mut series);
    assert!(series.points[0].partial && series.points[0].value == Some(1.25));
    store.push(
        2.0,
        SystemTime::UNIX_EPOCH,
        &[],
        &[],
        &HashMap::from([(t.clone(), vec![Some(3.0); 2])]),
    );
    store.record_partial(&[], &HashMap::from([(t.clone(), vec![Some(90.0); 2])]));
    store.copy_series(Some(&t), 0, &mut series);
    assert_eq!(series.points[1].value, Some(3.0));
    assert!(
        !series.points[1].partial,
        "known complete loads cannot be overwritten by a partial subtotal"
    );
    store.copy_series(LoadSource::Tracked, 0, &mut series);
    assert!(
        series.points[1].value.is_none(),
        "unknown rows never reuse an old partial value"
    );
}
use crate::app::shared::ids::{GroupId, RuleId};
use std::time::Duration;

#[test]
fn group_mean_retains_positive_partial_evidence_without_treating_gaps_as_zero() {
    let mut store = HistoryStore::default();
    store.configure(2, 30, &[]).unwrap();
    let time = SampleTime {
        seconds: 1.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    store.push_pending(time, &[]);
    assert!(store.complete(
        time,
        &super::super::model::Snapshot {
            cpus: vec![Some(90.0); 2],
            tracked_cpus: vec![Some(20.0), None],
            tracked_partial_cpus: vec![None, Some(10.0)],
            ..Default::default()
        }
    ));
    assert_eq!(
        store.group_mean_at(LoadSource::Tracked, time, &[0, 1, 1]),
        (Some(15.0), true)
    );
    assert_eq!(
        store.group_mean_at(LoadSource::Tracked, time, &[0]),
        (Some(20.0), false)
    );
    assert_eq!(
        store.group_mean_at(LoadSource::Tracked, time, &[2]),
        (None, false)
    );
    assert_eq!(
        store.group_mean_at(LoadSource::System, time, &[0, 1]),
        (Some(90.0), false)
    );
    assert_eq!(
        store.group_mean_at(
            LoadSource::Tracked,
            SampleTime {
                seconds: 2.0,
                ..time
            },
            &[0]
        ),
        (None, false)
    );
}

fn target() -> RuleTarget {
    RuleTarget {
        group: GroupId("g".into()),
        rule: RuleId("r".into()),
    }
}

#[test]
fn tracked_union_history_has_its_own_source_and_survives_retention_resize() {
    let mut store = HistoryStore::default();
    store.configure(2, 30, &[target()]).unwrap();
    let mut series = Series::default();
    for second in 1..=35 {
        store.push(
            second as f64,
            SystemTime::UNIX_EPOCH + Duration::from_secs(second),
            &[Some(90.0); 2],
            &[Some(false); 2],
            &HashMap::from([(target(), vec![Some(20.0); 2])]),
        );
        store.record_tracked(&[Some(40.0), None]);
    }
    store.copy_series(LoadSource::Tracked, 0, &mut series);
    assert_eq!(series.points.len(), 31);
    assert!(series.points.iter().all(|point| point.value == Some(40.0)));
    store.copy_series(None, 0, &mut series);
    assert_eq!(series.points[0].value, Some(90.0));
    store.copy_series(Some(&target()), 0, &mut series);
    assert_eq!(series.points[0].value, Some(20.0));
    let time = SampleTime {
        seconds: 10.0,
        wall: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
    };
    assert!(store.has_sample(LoadSource::Tracked, time, 1));
    assert!(store.value_at(LoadSource::Tracked, time, 1).is_none());
    store.configure(2, 300, &[target()]).unwrap();
    assert_eq!(store.value_at(LoadSource::Tracked, time, 0), Some(40.0));
    let bytes = store.reserved_bytes();
    store.push(
        36.0,
        SystemTime::UNIX_EPOCH,
        &[Some(90.0); 2],
        &[],
        &HashMap::new(),
    );
    assert!(
        store
            .value_at(
                LoadSource::Tracked,
                SampleTime {
                    seconds: 36.0,
                    wall: SystemTime::UNIX_EPOCH
                },
                0
            )
            .is_none(),
        "unrecorded tracked scope cannot reuse an overwritten row or system load"
    );
    store.record_tracked(&[Some(0.0); 2]);
    assert_eq!(
        store.reserved_bytes(),
        bytes,
        "recording overwrites reserved cells"
    );
}

#[test]
fn sample_membership_distinguishes_unknown_values_from_missing_source_or_time() {
    let mut store = HistoryStore::default();
    store.configure(2, 30, &[target()]).unwrap();
    let time = SampleTime {
        seconds: 1.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    store.push(
        time.seconds,
        time.wall,
        &[None; 2],
        &[],
        &HashMap::from([(target(), vec![None; 2])]),
    );
    assert!(store.has_sample(Some(&target()), time, 0));
    assert!(store.value_at(Some(&target()), time, 0).is_none());
    assert!(!store.has_sample(
        Some(&target()),
        SampleTime {
            seconds: 1.5,
            ..time
        },
        0
    ));
    assert!(!store.has_sample(Some(&target()), time, 2));
    store.configure(2, 30, &[]).unwrap();
    assert!(
        !store.has_sample(Some(&target()), time, 0),
        "retired source cannot authorize stale sparse rows"
    );
}

#[test]
fn overview_activity_tracks_only_positive_retained_samples_for_its_exact_rule() {
    let mut store = HistoryStore::default();
    let one = target();
    let two = RuleTarget {
        group: one.group.clone(),
        rule: RuleId("other".into()),
    };
    store.configure(4, 30, &[one.clone(), two.clone()]).unwrap();
    let mut overview = Overview::default();
    for second in 0..=31 {
        let values = if second == 0 {
            vec![Some(0.001), Some(0.0), None, Some(f32::NAN)]
        } else {
            vec![Some(0.0), Some(0.0), None, Some(0.0)]
        };
        store.push(
            second as f64,
            SystemTime::UNIX_EPOCH + Duration::from_secs(second),
            &[Some(99.0); 4],
            &[],
            &HashMap::from([
                (one.clone(), values),
                (two.clone(), vec![Some(0.0), Some(10.0), None, None]),
            ]),
        );
        store.copy_overview(Some(&one), &mut overview);
        assert_eq!(
            overview.activity,
            vec![second <= 30, false, false, false],
            "positive work lasts until that sample leaves the ring"
        );
    }
    let capacity = overview.activity.capacity();
    store.copy_overview(Some(&two), &mut overview);
    assert_eq!(
        overview.activity,
        vec![false, true, false, false],
        "do not reuse another app's activity"
    );
    assert_eq!(
        overview.activity.capacity(),
        capacity,
        "reuse the small per-CPU cache allocation"
    );
    store.push(
        32.0,
        SystemTime::UNIX_EPOCH,
        &[Some(99.0); 4],
        &[],
        &HashMap::from([(one.clone(), vec![None, None, Some(0.01), None])]),
    );
    store.copy_overview(Some(&one), &mut overview);
    assert_eq!(
        overview.activity,
        vec![false, false, true, false],
        "new app work adds its marker immediately"
    );
    store.configure(4, 30, &[two]).unwrap();
    store.copy_overview(Some(&one), &mut overview);
    assert!(
        overview.activity.is_empty(),
        "removed source cannot keep stale markers"
    );
    store.configure(4, 30, std::slice::from_ref(&one)).unwrap();
    store.copy_overview(Some(&one), &mut overview);
    assert_eq!(
        overview.activity,
        vec![false; 4],
        "re-added app has no old activity"
    );
}

#[test]
fn overview_activity_preserves_positive_evidence_inside_missing_or_gapped_buckets() {
    let mut store = HistoryStore::default();
    store.configure(2, 300, &[target()]).unwrap();
    for second in 0..=120 {
        let time = second as f64 + if second >= 46 { 4.0 } else { 0.0 };
        store.push(
            time,
            SystemTime::UNIX_EPOCH,
            &[Some(0.0); 2],
            &[],
            &HashMap::from([(
                target(),
                vec![
                    if second == 1 {
                        None
                    } else if second == 2 {
                        Some(0.01)
                    } else {
                        Some(0.0)
                    },
                    if second == 46 { Some(0.02) } else { Some(0.0) },
                ],
            )]),
        );
    }
    let mut overview = Overview::default();
    store.copy_overview(Some(&target()), &mut overview);
    assert!(
        overview.values.iter().any(Option::is_none),
        "mini graph must still preserve missing/gapped bins"
    );
    assert_eq!(
        overview.activity,
        vec![true, true],
        "known positive work must survive a different missing sample or time gap in its bucket"
    );
    store.configure(2, 30, &[target()]).unwrap();
    store.copy_overview(Some(&target()), &mut overview);
    assert_eq!(
        overview.activity,
        vec![false, false],
        "shorter retention removes all expired evidence"
    );
}

#[test]
fn historical_sample_uses_one_exact_time_for_all_cpus_and_preserves_unknowns() {
    let mut store = HistoryStore::default();
    store.configure(3, 30, &[target()]).unwrap();
    for second in 1..=35 {
        store.push(
            second as f64,
            SystemTime::UNIX_EPOCH + Duration::from_secs(second),
            &[Some(90.0); 3],
            &[Some(true), None, Some(false)],
            &HashMap::from([(target(), vec![Some(second as f32), None, Some(70.0)])]),
        );
    }
    let mut sample = HistoricalSample::default();
    store.copy_sample(Some(&target()), 12.0, &mut sample);
    assert_eq!(sample.time.unwrap().seconds, 12.0);
    assert_eq!(sample.values, vec![Some(12.0), None, Some(70.0)]);
    assert_eq!(sample.parked, vec![Some(true), None, Some(false)]);
    let capacity = sample.values.capacity();
    store.copy_sample(None, 13.0, &mut sample);
    assert_eq!(sample.values, vec![Some(90.0); 3]);
    assert_eq!(sample.values.capacity(), capacity);
    store.copy_sample(Some(&target()), 1.0, &mut sample);
    assert!(
        sample.time.is_none(),
        "evicted samples must not become newer samples"
    );
    assert!(sample.values.iter().all(Option::is_none));
    store.copy_sample(Some(&target()), 12.5, &mut sample);
    assert!(
        sample.time.is_none(),
        "do not interpolate or mix sample times"
    );
    store.configure(3, 30, &[]).unwrap();
    store.copy_sample(Some(&target()), 12.0, &mut sample);
    assert!(
        sample.time.is_none(),
        "deleted rules must not keep stale values"
    );
}

#[test]
fn mini_overview_is_bounded_preserves_peaks_gaps_and_rule_addition_boundary() {
    let mut store = HistoryStore::default();
    store.configure(2, 1800, &[]).unwrap();
    store.push(
        0.0,
        SystemTime::UNIX_EPOCH,
        &[Some(99.0); 2],
        &[],
        &HashMap::new(),
    );
    store.configure(2, 1800, &[target()]).unwrap();
    for second in 1..=1800 {
        let load = if second == 777 { 100.0 } else { 2.0 };
        store.push(
            second as f64,
            SystemTime::UNIX_EPOCH,
            &[Some(99.0); 2],
            &[],
            &HashMap::from([(
                target(),
                vec![Some(load), if second == 900 { None } else { Some(7.0) }],
            )]),
        );
    }
    let mut overview = Overview::default();
    store.copy_overview(Some(&target()), &mut overview);
    assert!(overview.times.len() <= 64);
    assert_eq!(overview.values.len(), overview.times.len() * 2);
    assert!(
        overview.times[0].0 >= 1.0,
        "new apps have no pre-addition data"
    );
    let bins = overview.times.len();
    assert!(overview.values[..bins]
        .iter()
        .flatten()
        .any(|&(_, max)| max == 100.0));
    assert!(
        overview.values[bins..].iter().any(Option::is_none),
        "unknown samples remain a gap"
    );
    let capacity = overview.values.capacity();
    store.copy_overview(Some(&target()), &mut overview);
    assert_eq!(overview.values.capacity(), capacity);
}
