use super::*;

#[cfg(feature = "debug-mcp")]
#[test]
fn debug_disabled_acknowledgement_requires_the_worker_release_revision() {
    let runtime = CpuLoadRuntime::new(
        Default::default(),
        false,
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
    );
    {
        let mut control = runtime.control.0.lock().unwrap();
        control.enabled = false;
        control.revision = 8;
    }
    assert_eq!(runtime.debug_disabled_acknowledged(), Some(false));
    *runtime.published.write().unwrap() = Arc::new(Snapshot {
        revision: 8,
        ..Default::default()
    });
    assert_eq!(runtime.debug_disabled_acknowledged(), Some(true));
    *runtime.published.write().unwrap() = Arc::new(Snapshot {
        revision: 8,
        error: Some("trace cleanup failed".into()),
        ..Default::default()
    });
    assert_eq!(runtime.debug_disabled_acknowledged(), Some(false));
}
use crate::app::shared::ids::{GroupId, RuleId};

#[test]
fn transient_membership_race_does_not_erase_the_next_valid_cpu_target_set() {
    let expected = vec![(10, 100), (20, 200), (30, 300)];
    let all = expected.clone();
    let mut fallback = Vec::new();
    let runs = HashMap::from([("run".into(), (30, 300))]);
    assert_eq!(
        history_tick_targets(&all, false, &runs, &mut fallback),
        &[(30, 300)]
    );
    assert_eq!(
        history_tick_targets(&all, true, &runs, &mut fallback),
        expected,
        "a one-tick lock/generation race must not permanently remove saved app PIDs"
    );
}

#[test]
fn s92_membership_race_recovers_app_group_history_contributors_and_current_api() {
    use super::super::worker_history::{EventHistory, OwnerInput};
    use os_api::cpu_load::{CpuHistoryFrame, CpuHistoryTarget, CpuHistoryTick, CpuHistoryValue};
    let runtime = runtime();
    let target = target();
    let all = vec![(10, 100), (20, 200)];
    let targets = HashMap::from([(target.clone(), all.clone())]);
    let names: HashMap<_, _> = all
        .iter()
        .map(|&(pid, token)| {
            (
                (pid, token),
                Arc::new(os_api::cpu_load::CpuProcess {
                    pid,
                    instance: token,
                    name: if pid == 10 {
                        "launcher.exe"
                    } else {
                        "added-game.exe"
                    }
                    .into(),
                }),
            )
        })
        .collect();
    let rules = HashMap::from([(target.clone(), Arc::new(target.rule.clone()))]);
    let runs = HashMap::new();
    let mut scratch = Vec::new();
    let mut previous = Vec::new();
    let mut events = EventHistory::default();
    let mut store = runtime.history.write().unwrap();
    store
        .configure(2, 30, std::slice::from_ref(&target))
        .unwrap();
    for id in 0..=4 {
        let valid = id != 2;
        let selected = history_tick_targets(&all, valid, &runs, &mut scratch);
        events
            .enqueue(
                CpuHistoryTick {
                    id,
                    seconds: id as f64,
                    timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(id),
                },
                OwnerInput {
                    generation: valid.then_some(1),
                    targets: valid.then_some(&targets),
                    names: &names,
                    rules: &rules,
                    runs: &runs,
                    changed: false,
                },
                &[],
                &mut store,
            )
            .unwrap();
        let mut frame = CpuHistoryFrame::new(2).unwrap();
        frame.id = id;
        frame.available = id != 0;
        frame.cpus = vec![Some(10.0), Some(80.0)];
        for (index, &(pid, token)) in previous.iter().enumerate() {
            frame.targets.push(CpuHistoryTarget { pid, token });
            frame.coverage.push(3);
            frame.tracked.push(CpuHistoryValue {
                target: index as u16,
                cpu: if pid == 10 { 0 } else { 1 },
                load: if pid == 10 { 10.0 } else { 80.0 },
            });
        }
        events.apply(&frame, &mut store);
        previous = selected.to_vec();
    }
    let mut snapshot = Snapshot {
        cpus: vec![Some(10.0), Some(80.0)],
        ..Default::default()
    };
    events.publish(&mut snapshot);
    let totals = HashMap::from([((10, 100), Some(5.0)), ((20, 200), Some(40.0))]);
    snapshot.processes = process_totals(&targets, |pid, token| totals[&(pid, token)]);
    let mut current =
        super::super::process_list::ProcessList::from_tracking(&targets, &totals, &names);
    current.record_cores(2, events.samples());
    snapshot.live_processes = Arc::new(current);
    assert_eq!(snapshot.processes[&target], Some(45.0));
    assert_eq!(snapshot.process_cpus[&target], [Some(10.0), Some(80.0)]);
    let recovered = SampleTime {
        seconds: 4.0,
        wall: SystemTime::UNIX_EPOCH + Duration::from_secs(4),
    };
    assert_eq!(
        store.group_mean_at(LoadSource::Tracked, recovered, &[0, 1]),
        (Some(45.0), false)
    );
    assert_eq!(
        store.value_at(LoadSource::Rule(&target), recovered, 1),
        Some(80.0)
    );
    let mut contributors = super::super::contributors::ContributorView::default();
    store
        .contributors
        .copy(recovered, 1, Some(&target.rule), &mut contributors);
    assert_eq!(contributors.rows.len(), 1);
    assert_eq!(contributors.rows[0].process.pid, 20);
    assert_eq!(contributors.rows[0].load, 80.0);
    assert_eq!(
        store.value_at(
            LoadSource::Tracked,
            SampleTime {
                seconds: 3.0,
                wall: SystemTime::UNIX_EPOCH + Duration::from_secs(3)
            },
            1
        ),
        None,
        "the uncertain original interval must remain a gap"
    );
    drop(store);
    *runtime.published.write().unwrap() = Arc::new(snapshot);
    let api = runtime
        .read_handle()
        .read("read_cpu_current", serde_json::json!({}))
        .unwrap();
    assert_eq!(
        api["tracked_etw_residency_percent"],
        serde_json::json!([10.0, 80.0])
    );
    assert!(api["current_processes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["pid"] == 20
            && row["whole_host_cpu_percent"] == 40.0
            && row["active_cpus"] == serde_json::json!([[1, 80.0]])));
}

#[test]
fn group_meter_uses_latest_finalized_tracked_interval_and_keeps_missing_data_unknown() {
    let runtime = runtime();
    {
        let mut history = runtime.history.write().unwrap();
        history.configure(2, 30, &[]).unwrap();
        history.push(
            1.0,
            SystemTime::UNIX_EPOCH,
            &[Some(90.0); 2],
            &[],
            &HashMap::new(),
        );
        history.record_tracked(&[Some(20.0), Some(40.0)]);
        history.push_pending(
            SampleTime {
                seconds: 2.0,
                wall: SystemTime::UNIX_EPOCH,
            },
            &[],
        );
    }
    assert_eq!(runtime.tracked_group_load(&[0, 1, 1]), (Some(30.0), false));
    assert_eq!(runtime.tracked_group_load(&[]), (None, false));
    {
        let mut history = runtime.history.write().unwrap();
        history.mark_lost_reason(
            SampleTime {
                seconds: 2.0,
                wall: SystemTime::UNIX_EPOCH,
            },
            super::super::history::GapReason::TraceUnavailable,
        );
    }
    assert_eq!(runtime.tracked_group_load(&[0, 1]), (None, false));
    let _writer = runtime.history.write().unwrap();
    assert_eq!(runtime.tracked_group_load(&[0, 1]), (None, false));
}

#[test]
fn system_modal_opens_all_cpus_and_all_processes_without_a_saved_group() {
    let mut runtime = runtime();
    runtime.open(GraphTarget::System);
    assert!(runtime.show_all_cpus);
    assert!(!runtime.tracked_only());
    assert!(!runtime.show_processes);
    assert!(runtime.graph == Some(GraphTarget::System));
}

#[test]
fn group_modal_opens_tracked_only_and_resets_process_view_on_reopen() {
    let mut runtime = runtime();
    let group = GraphTarget::Group {
        id: GroupId("g".into()),
        name: "Group".into(),
        cpus: vec![0],
    };
    runtime.open(group.clone());
    assert!(runtime.tracked_only());
    runtime.show_processes = true;
    runtime.set_tracked_only(false);
    runtime.open(group);
    assert!(runtime.tracked_only());
    assert!(!runtime.show_processes);
    runtime.open(GraphTarget::Process {
        target: target(),
        name: "App".into(),
    });
    assert!(!runtime.tracked_only());
}

#[test]
fn delayed_frame_completes_original_graph_and_contributors_together() {
    use super::super::worker_history::{EventHistory, OwnerInput};
    use os_api::cpu_load::{
        CpuHistoryContribution, CpuHistoryFrame, CpuHistoryTarget, CpuHistoryTick, CpuHistoryValue,
    };
    let mut runtime = runtime();
    let t = target();
    let mut events = EventHistory::default();
    let process = Arc::new(os_api::cpu_load::CpuProcess {
        pid: 42,
        instance: 10,
        name: "old.exe".into(),
    });
    let names = HashMap::from([((42, 10), process.clone())]);
    let rules = HashMap::from([(t.clone(), Arc::new(t.rule.clone()))]);
    let targets = HashMap::from([(t.clone(), vec![(42, 10)])]);
    let tick = |id| CpuHistoryTick {
        id,
        seconds: id as f64 + 10.0,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(id),
    };
    let runs = HashMap::new();
    let input = || OwnerInput {
        runs: &runs,
        generation: Some(1),
        targets: Some(&targets),
        names: &names,
        rules: &rules,
        changed: false,
    };
    {
        let mut store = runtime.history.write().unwrap();
        store.configure(1, 30, std::slice::from_ref(&t)).unwrap();
        events.enqueue(tick(0), input(), &[], &mut store).unwrap();
        let mut first = CpuHistoryFrame::new(1).unwrap();
        first.id = 0;
        events.apply(&first, &mut store);
        events.enqueue(tick(1), input(), &[], &mut store).unwrap();
        events.enqueue(tick(2), input(), &[], &mut store).unwrap();
    }
    let time = SampleTime {
        seconds: 11.0,
        wall: tick(1).timestamp,
    };
    assert!(runtime
        .graph_data(Some(&t), 0, |_| Some(time))
        .contributors
        .rows
        .is_empty());
    {
        let mut frame = CpuHistoryFrame::new(1).unwrap();
        frame.id = 1;
        frame.available = true;
        frame.cpus[0] = Some(50.0);
        frame.targets.push(CpuHistoryTarget { pid: 42, token: 10 });
        frame.coverage.push(1);
        frame.tracked.push(CpuHistoryValue {
            target: 0,
            cpu: 0,
            load: 50.0,
        });
        frame.contributors.push(CpuHistoryContribution {
            process: process.clone(),
            cpu: 0,
            load: 50.0,
        });
        events.apply(&frame, &mut runtime.history.write().unwrap());
    }
    let data = runtime.graph_data(Some(&t), 0, |_| Some(time));
    assert_eq!(data.sample.values, [Some(50.0)]);
    assert_eq!(data.contributors.time, Some(time));
    assert_eq!(data.contributors.rows[0].load, 50.0);
    assert!(
        data.series.points.last().unwrap().value.is_none(),
        "the newest provisional interval must not receive an old burst"
    );
    let live = runtime.graph_data(Some(&t), 0, |_| None);
    assert_eq!(
        live.sample.time,
        Some(time),
        "Live tiles and rows must select the latest completed original sample"
    );
    assert_eq!(live.contributors.time, Some(time));
    events.fail(&mut runtime.history.write().unwrap());
    let lost = runtime.graph_data(Some(&t), 0, |_| None);
    assert_eq!(
        lost.sample.time.unwrap().seconds,
        12.0,
        "Live must expose the latest lost interval instead of stale completed work"
    );
    assert_eq!(lost.sample.values, [None]);
    assert!(!lost.contributors.available);
}

#[test]
fn group_tracked_filter_changes_every_data_source_and_deduplicates_historical_pids() {
    let mut runtime = runtime();
    let one = target();
    let two = RuleTarget {
        rule: RuleId("other".into()),
        ..one.clone()
    };
    let time = SampleTime {
        seconds: 1.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    let process = |pid, load, rule: Option<&RuleId>| Contribution {
        run: None,
        process: Arc::new(os_api::cpu_load::CpuProcess {
            pid,
            instance: 100,
            name: format!("p{pid}.exe").into(),
        }),
        rule: rule.map(|r| Arc::new(r.clone())),
        cpu: 0,
        load,
    };
    {
        let mut history = runtime.history.write().unwrap();
        history
            .configure(2, 30, &[one.clone(), two.clone()])
            .unwrap();
        history.push(
            time.seconds,
            time.wall,
            &[Some(90.0); 2],
            &[],
            &HashMap::from([
                (one.clone(), vec![Some(20.0); 2]),
                (two.clone(), vec![Some(30.0); 2]),
            ]),
        );
        history.record_tracked(&[Some(30.0), None]);
        history.contributors.push(
            time,
            &[
                process(1, 20.0, None),
                process(2, 50.0, None),
                process(1, 20.0, Some(&one.rule)),
                process(1, 20.0, Some(&two.rule)),
                process(3, 10.0, Some(&two.rule)),
            ],
            true,
        );
    }
    let all = runtime.graph_data(None, 0, |_| Some(time));
    assert_eq!(all.series.points[0].value, Some(90.0));
    assert!(all.contributors.rows.iter().any(|r| r.process.pid == 2));
    runtime.set_tracked_only(true);
    let tracked = runtime.graph_data(None, 0, |_| Some(time));
    assert_eq!(tracked.series.points[0].value, Some(30.0));
    assert_eq!(tracked.sample.values, vec![Some(30.0), None]);
    assert_eq!(tracked.contributors.rows.len(), 2);
    assert_eq!(
        tracked
            .contributors
            .rows
            .iter()
            .map(|r| r.load)
            .sum::<f32>(),
        30.0,
        "one shared PID is counted once across saved rules"
    );
    assert!(!tracked.contributors.rows.iter().any(|r| r.process.pid == 2));
    assert!(
        !runtime
            .graph_data(None, 1, |_| Some(time))
            .contributors
            .available
    );
    runtime.set_tracked_only(false);
    assert_eq!(
        runtime.graph_data(None, 0, |_| Some(time)).series.points[0].value,
        Some(90.0),
        "cached tracked values must not leak into all-process scope"
    );
    runtime.set_tracked_only(true);
    assert_eq!(
        runtime
            .graph_data(Some(&one), 0, |_| Some(time))
            .series
            .points[0]
            .value,
        Some(20.0),
        "app graphs retain their exact app scope"
    );
    runtime.close();
    assert!(!runtime.tracked_only());
}

#[test]
fn unknown_app_totals_do_not_hide_exact_measured_process_contributors() {
    let mut runtime = runtime();
    let target = target();
    let time = SampleTime {
        seconds: 1.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    {
        let mut history = runtime.history.write().unwrap();
        history
            .configure(2, 30, std::slice::from_ref(&target))
            .unwrap();
        history.push(
            time.seconds,
            time.wall,
            &[Some(50.0); 2],
            &[],
            &HashMap::from([(target.clone(), vec![None; 2])]),
        );
        history.contributors.push(
            time,
            &[Contribution {
                run: None,
                process: Arc::new(os_api::cpu_load::CpuProcess {
                    pid: 42,
                    instance: 100,
                    name: "busy.exe".into(),
                }),
                rule: Some(Arc::new(target.rule.clone())),
                cpu: 0,
                load: 40.0,
            }],
            true,
        );
    }
    let data = runtime.graph_data(Some(&target), 0, |_| Some(time));
    assert!(
        data.series.points[0].value.is_none(),
        "unknown app aggregate must stay unknown"
    );
    assert_eq!(
        data.contributors.rows.len(),
        1,
        "a known busy PID must remain visible despite an unavailable sibling"
    );
    assert_eq!(data.contributors.rows[0].process.pid, 42);
    assert!(
        data.contributors.partial,
        "known contributors cannot be presented as a complete aggregate"
    );
    assert!(
        !runtime
            .graph_data(Some(&target), 1, |_| Some(time))
            .contributors
            .available,
        "unknown without measured contributors is not an idle interval"
    );
}

#[test]
fn missing_siblings_preserve_positive_measured_subtotal_without_inventing_idle() {
    let samples = vec![os_api::cpu_load::ProcessCpuSample {
        pid: 2,
        token: 20,
        cpus: vec![Some(1.25), Some(0.0), None],
    }];
    let mut full = vec![None; 3];
    let mut partial = vec![None; 3];
    aggregate_cpus(&mut full, &mut partial, &[(1, 10), (2, 20)], &samples);
    assert_eq!(
        full,
        vec![None; 3],
        "a missing sibling still prevents a complete total"
    );
    assert_eq!(
        partial,
        vec![Some(1.25), None, None],
        "known positive work survives missing siblings, unknown zero is not idle"
    );
    aggregate_cpus(&mut full, &mut partial, &[(2, 20)], &samples);
    assert_eq!(full, vec![Some(1.25), Some(0.0), None]);
    assert!(
        partial.iter().all(Option::is_none),
        "complete values need no partial duplicate"
    );
    aggregate_cpus(&mut full, &mut partial, &[(2, 21)], &samples);
    assert!(
        full.iter().chain(&partial).all(Option::is_none),
        "PID reuse cannot supply stale work"
    );
}

#[test]
fn process_aggregation_sums_exact_instances_once_and_preserves_missing_cpus() {
    use os_api::cpu_load::ProcessCpuSample;
    let samples = vec![
        ProcessCpuSample {
            pid: 1,
            token: 10,
            cpus: vec![Some(20.0), None],
        },
        ProcessCpuSample {
            pid: 2,
            token: 20,
            cpus: vec![Some(30.0), Some(40.0)],
        },
    ];
    let mut values = vec![None; 2];
    let mut partial = vec![None; 2];
    aggregate_cpus(&mut values, &mut partial, &[(1, 10), (2, 20)], &samples);
    assert_eq!(values, vec![Some(50.0), None]);
    aggregate_cpus(&mut values, &mut partial, &[(1, 11)], &samples);
    assert_eq!(
        values,
        vec![None, None],
        "a reused PID must not inherit old token data"
    );
}
fn runtime() -> CpuLoadRuntime {
    CpuLoadRuntime::new(
        Arc::new(TokioRwLock::new(RunningApps::default())),
        false,
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
    )
}

#[test]
fn memory_snapshot_rejects_stale_control_revisions_and_monitoring_off() {
    let mut runtime = runtime();
    let rule = target();
    let usage = super::super::memory::MemoryUsage {
        bytes: Some(256 * 1024 * 1024),
        measured: 2,
        instances: 2,
    };
    {
        let mut control = runtime.control.0.lock().unwrap();
        control.enabled = true;
        control.revision = 7;
    }
    *runtime.published.write().unwrap() = Arc::new(Snapshot {
        revision: 7,
        process_memory: HashMap::from([(rule.clone(), usage)]),
        group_memory: Some(HashMap::from([(rule.group.clone(), usage)])),
        ..Default::default()
    });
    assert_eq!(runtime.snapshot().process_memory.get(&rule), Some(&usage));
    assert_eq!(
        runtime
            .snapshot()
            .group_memory
            .as_ref()
            .unwrap()
            .get(&rule.group),
        Some(&usage)
    );
    *runtime.published.write().unwrap() = Arc::new(Snapshot {
        revision: 7,
        at: Instant::now() - Duration::from_secs(5),
        process_memory: HashMap::from([(rule.clone(), usage)]),
        group_memory: Some(HashMap::from([(rule.group.clone(), usage)])),
        ..Default::default()
    });
    assert!(runtime.snapshot().process_memory.is_empty());
    assert!(runtime.snapshot().group_memory.is_none());
    *runtime.published.write().unwrap() = Arc::new(Snapshot {
        revision: 6,
        process_memory: HashMap::from([(rule.clone(), usage)]),
        group_memory: Some(HashMap::from([(rule.group.clone(), usage)])),
        ..Default::default()
    });
    assert!(runtime.snapshot().process_memory.is_empty());
    assert!(runtime.snapshot().group_memory.is_none());
    *runtime.published.write().unwrap() = Arc::new(Snapshot {
        revision: 7,
        group_memory: Some(HashMap::from([(rule.group, usage)])),
        ..Default::default()
    });
    assert!(runtime.snapshot().group_memory.is_some());
    runtime.configure(false, false);
    assert!(runtime.snapshot().process_memory.is_empty());
    assert!(runtime.snapshot().group_memory.is_none());
}
#[test]
fn graph_contributors_follow_exact_hover_cpu_and_never_fall_back_to_live_rows() {
    let mut runtime = runtime();
    let target = target();
    let one = SampleTime {
        seconds: 1.0,
        wall: SystemTime::UNIX_EPOCH,
    };
    let two = SampleTime {
        seconds: 2.0,
        ..one
    };
    let process = Arc::new(os_api::cpu_load::CpuProcess {
        pid: 42,
        instance: 100,
        name: "old.exe".into(),
    });
    {
        let mut history = runtime.history.write().unwrap();
        history
            .configure(2, 30, std::slice::from_ref(&target))
            .unwrap();
        for time in [one, two] {
            history.push(
                time.seconds,
                time.wall,
                &[Some(50.0); 2],
                &[],
                &HashMap::from([(target.clone(), vec![Some(20.0); 2])]),
            );
            let inputs = vec![
                Contribution {
                    run: None,
                    process: process.clone(),
                    rule: None,
                    cpu: 0,
                    load: 10.0 * time.seconds as f32,
                },
                Contribution {
                    run: None,
                    process: process.clone(),
                    rule: Some(Arc::new(target.rule.clone())),
                    cpu: 1,
                    load: 20.0,
                },
            ];
            history.contributors.push(time, &inputs, true);
        }
    }
    assert_eq!(
        runtime.graph_data(None, 0, |_| None).contributors.rows[0].load,
        20.0
    );
    assert_eq!(
        runtime.graph_data(None, 0, |_| Some(one)).contributors.rows[0].load,
        10.0
    );
    assert!(runtime
        .graph_data(None, 1, |_| Some(one))
        .contributors
        .rows
        .is_empty());
    assert_eq!(
        runtime
            .graph_data(Some(&target), 1, |_| Some(one))
            .contributors
            .rows[0]
            .load,
        20.0
    );
    let missing = SampleTime {
        seconds: 1.5,
        ..one
    };
    assert!(
        !runtime
            .graph_data(None, 0, |_| Some(missing))
            .contributors
            .available
    );
    let history = runtime.history.clone();
    let _guard = history.write().unwrap();
    assert!(
        !runtime
            .graph_data(Some(&target), 0, |_| Some(one))
            .contributors
            .available
    );
}
fn target() -> RuleTarget {
    RuleTarget {
        group: GroupId("g".into()),
        rule: RuleId("r".into()),
    }
}

#[test]
fn quick_disable_enable_still_starts_a_fresh_collection() {
    let previous = Control {
        enabled: true,
        revision: 1,
        ..Control::default()
    };
    let current = Control {
        enabled: true,
        revision: 3,
        ..Control::default()
    };
    assert!(
        collection_reset_required(&current, &previous),
        "coalescing off/on cannot retain the previous collection"
    );
    let retention = Control {
        history_seconds: 30,
        ..previous.clone()
    };
    assert!(
        !collection_reset_required(&retention, &previous),
        "retention must preserve history"
    );
}

#[test]
fn contended_history_never_displays_the_previous_cpu_under_a_new_label() {
    let mut runtime = runtime();
    runtime
        .history
        .write()
        .unwrap()
        .configure(2, 30, &[target()])
        .unwrap();
    runtime.history.write().unwrap().push(
        1.0,
        SystemTime::UNIX_EPOCH,
        &[Some(10.0), Some(90.0)],
        &[],
        &HashMap::new(),
    );
    assert_eq!(runtime.series(None, 0).points[0].value, Some(10.0));
    let history = runtime.history.clone();
    let _writer = history.write().unwrap();
    assert!(
        runtime.series(None, 1).points.is_empty(),
        "must show no data while waiting, never CPU0 data as CPU1"
    );
}
#[test]
#[ignore = "explicit elevated read-only load generator for the CPU cost probe"]
fn native_cpu_cost_load_generator() {
    let until = Instant::now() + Duration::from_secs(100);
    // Exercise frequent switches without heating eight more CPU cores.
    let sleepers: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(move || {
                while Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        })
        .collect();
    let mut value = 1u64;
    while Instant::now() < until {
        for _ in 0..10_000 {
            value = std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(1));
        }
    }
    std::hint::black_box(value);
    for sleeper in sleepers {
        sleeper.join().unwrap();
    }
}

#[test]
#[ignore = "explicit elevated continuous CPU history/overhead probe; about 55 seconds, no OS settings changed"]
fn native_continuous_history_cost_and_lifecycle() {
    use crate::app::features::rules::RulesContext;
    use crate::app::models::{AppToRun, CoreGroup, RunningApp};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    struct Load(Child);
    impl Drop for Load {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let executable = std::env::current_exe().unwrap();
    let load = Load(
        Command::new(&executable)
            .args([
                "--ignored",
                "--exact",
                "app::features::cpu_load::runtime::tests::native_cpu_cost_load_generator",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let pid = load.0.id();
    let token = os_api::OS::get_process_instance_token(pid).unwrap();
    let cpus = os_api::cpu_load::logical_cpu_count();
    let programs: Vec<_> = (0..10)
        .map(|n| {
            AppToRun::new_path(
                executable.clone(),
                vec![format!("cost-{n}")],
                executable.clone(),
                os_api::PriorityClass::Normal,
                false,
            )
        })
        .collect();
    let mut state = AppStateStorage::test_fixture(|s| {
        s.cpu_load_history_seconds = 1800;
        s.groups = vec![CoreGroup {
            name: "Probe".into(),
            cores: (0..cpus).collect(),
            programs: programs.clone(),
            is_hidden: false,
            run_all_button: false,
        }];
        s.rule_identities = None;
    });
    let ids = RulesContext::from_storage(&state).to_persisted_identities();
    let group = ids.groups[0].id.clone();
    let first = RuleTarget {
        group: group.clone(),
        rule: ids.groups[0].rule_ids[0].clone(),
    };
    let mut running = RunningApps::default();
    for (app, rule) in programs.iter().zip(&ids.groups[0].rule_ids) {
        running.apps.insert(
            app.runtime_key(),
            RunningApp {
                pids: vec![pid],
                pid_instance_tokens: HashMap::from([(pid, token)]),
                group_id: group.clone(),
                rule_id: rule.clone(),
                created_at: SystemTime::now(),
                settings_matched: true,
            },
        );
    }
    state.rule_identities = Some(ids);
    let source = Arc::new(RwLock::new(state));
    let running = Arc::new(TokioRwLock::new(running));
    struct ProbeRuntime(CpuLoadRuntime);
    impl std::ops::Deref for ProbeRuntime {
        type Target = CpuLoadRuntime;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl std::ops::DerefMut for ProbeRuntime {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }
    impl Drop for ProbeRuntime {
        fn drop(&mut self) {
            self.0.shutdown();
            let until = Instant::now() + Duration::from_secs(8);
            while !self.0.is_stopped() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut runtime = ProbeRuntime(CpuLoadRuntime::new(running.clone(), true, source.clone()));
    runtime.set_history_seconds(1800);
    fn measure(seconds: u64) -> (f64, usize, usize, u64) {
        let before = os_api::cpu_load::resource_sample().unwrap();
        let start = Instant::now();
        std::thread::sleep(Duration::from_secs(seconds));
        let after = os_api::cpu_load::resource_sample().unwrap();
        let one_cpu_percent = (after.cpu_ticks - before.cpu_ticks) as f64
            / 10_000_000.0
            / start.elapsed().as_secs_f64()
            * 100.0;
        (
            one_cpu_percent,
            after.private_bytes,
            after.working_set,
            after.cycles - before.cycles,
        )
    }
    let baseline = measure(10);
    runtime.configure(true, false);
    std::thread::sleep(Duration::from_secs(5));
    let before = runtime.snapshot();
    assert!(before.error.is_none(), "{:?}", before.error);
    assert!(before.detail_error.is_none(), "{:?}", before.detail_error);
    assert!(
        before.process_cpus[&first]
            .iter()
            .any(|v| v.is_some_and(|v| v > 1.0)),
        "no real per-CPU residency"
    );
    assert_eq!(
        before.instances, 1,
        "ten app histories share one exact sampled PID"
    );
    let enabled = measure(20);
    assert_eq!(
        runtime.snapshot().history_bytes,
        before.history_bytes,
        "history must not allocate as it fills"
    );
    let selected = before.process_cpus[&first]
        .iter()
        .position(|v| v.is_some_and(|v| v > 1.0))
        .unwrap();
    let data = runtime.graph_data(Some(&first), selected, |_| None);
    assert!(
        data.contributors.available,
        "app process contributors unavailable"
    );
    assert!(data
        .contributors
        .rows
        .iter()
        .any(|row| row.process.pid == pid && row.process.instance == token));
    let sum: f32 = data.contributors.rows.iter().map(|r| r.load).sum();
    assert!(
        (sum - data.sample.values[selected].unwrap()).abs() < 0.01,
        "app row contributions must match its aggregate sample"
    );
    let live_time = data.sample.time.unwrap();
    assert_eq!(data.contributors.time, Some(live_time));
    assert_eq!(
        data.series
            .points
            .iter()
            .find(|p| p.seconds == live_time.seconds)
            .unwrap()
            .value,
        Some(sum)
    );
    assert!(data.series.points.last().unwrap().seconds >= live_time.seconds);
    // Exercise immutable graph reads and exact-time inspection while collection
    // continues. This includes overview/history copying, not a GPU benchmark.
    let before_reads = os_api::cpu_load::resource_sample().unwrap();
    let read_start = Instant::now();
    for index in 0..2000 {
        let cpu = index % cpus;
        let data = runtime.graph_data(Some(&first), cpu, |_| {
            if index % 2 == 0 {
                Some(live_time)
            } else {
                None
            }
        });
        assert_eq!(data.contributors.time, data.sample.time);
    }
    let after_reads = os_api::cpu_load::resource_sample().unwrap();
    eprintln!(
        "GRAPH_READS count=2000 elapsed_ms={:.2} cpu_ms={:.2} private_delta_bytes={}",
        read_start.elapsed().as_secs_f64() * 1000.0,
        (after_reads.cpu_ticks - before_reads.cpu_ticks) as f64 / 10000.0,
        after_reads.private_bytes as i64 - before_reads.private_bytes as i64
    );
    let data = runtime.graph_data(None, selected, |_| None);
    assert!(
        data.contributors.available,
        "host process contributors unavailable"
    );
    assert!(
        data.contributors
            .rows
            .iter()
            .any(|row| row.process.pid != pid),
        "system history must include unconfigured processes"
    );
    let short = Load(
        Command::new(&executable)
            .args([
                "--ignored",
                "--exact",
                "app::features::cpu_load::runtime::tests::native_cpu_cost_load_generator",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let short_pid = short.0.id();
    std::thread::sleep(Duration::from_millis(400));
    drop(short);
    std::thread::sleep(Duration::from_secs(2));
    let mut found_exited = false;
    // Search original sampled rows, including the final interval of a process
    // which started AND exited between our one-second polls.
    let series = runtime
        .series(None, 0)
        .points
        .iter()
        .map(|p| SampleTime {
            seconds: p.seconds,
            wall: p.timestamp,
        })
        .collect::<Vec<_>>();
    for time in series.iter().rev().take(5) {
        for cpu in 0..cpus {
            let data = runtime.graph_data(None, cpu, |_| Some(*time));
            found_exited |= data
                .contributors
                .rows
                .iter()
                .any(|r| r.process.pid == short_pid);
        }
    }
    assert!(
        found_exited,
        "short-lived exited process must remain named in history"
    );
    eprintln!(
        "CONTRIBUTORS selected_cpu={selected} short_lived_retained={found_exited} identities={} estimated={}",
        runtime.snapshot().contributor_identities,
        runtime.memory_estimate(cpus, 10, 1800)
    );
    runtime.open(GraphTarget::Process {
        target: first.clone(),
        name: "Probe".into(),
    });
    assert!(
        runtime.series(Some(&first), 0).points.len() >= 20,
        "history must predate opening graph"
    );
    let recorded_samples = runtime.series(Some(&first), 0).points.len();
    runtime.close();
    runtime.configure(true, false);
    let original = runtime.snapshot().history_bytes;
    {
        let mut source = source.write().unwrap();
        source.groups[0].programs.push(programs[0].clone());
        source.rule_identities.as_mut().unwrap().groups[0]
            .rule_ids
            .push(RuleId("probe-new".into()));
    }
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        runtime.snapshot().history_bytes,
        original + 1801 * cpus * 5,
        "new stopped app must get a reserved ring"
    );
    let added = RuleTarget {
        group: group.clone(),
        rule: RuleId("probe-new".into()),
    };
    let new_series = runtime.series(Some(&added), 0);
    assert!(
        new_series.points.len() <= 3 && new_series.points.iter().all(|p| p.value.is_none()),
        "no fabricated pre-addition history"
    );
    {
        let mut source = source.write().unwrap();
        source.groups[0].programs.pop();
        source.rule_identities.as_mut().unwrap().groups[0]
            .rule_ids
            .pop();
    }
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        runtime.snapshot().history_bytes,
        original,
        "deleted app history must be released"
    );
    assert!(
        runtime.series(Some(&first), 0).points.len() >= 20,
        "other histories survive app changes"
    );
    runtime.configure(false, false);
    let until = Instant::now() + Duration::from_secs(8);
    while runtime.history.read().unwrap().reserved_bytes() != 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        runtime.history.read().unwrap().reserved_bytes(),
        0,
        "disable releases all history buffers"
    );
    let disabled = measure(10);
    // A group graph still needs host contributors when no app rules exist.
    {
        let mut source = source.write().unwrap();
        source.groups.clear();
        source.rule_identities.as_mut().unwrap().groups.clear();
    }
    runtime.configure(true, false);
    let until = Instant::now() + Duration::from_secs(8);
    let mut ready = false;
    while Instant::now() < until {
        // A first PDH interval can be complete before ETW has observed a
        // switch on that particular CPU. Wait for an actually recorded host
        // contributor row, rather than treating its valid initial None as zero.
        if runtime.snapshot().contributor_identities > 0 {
            ready = (0..cpus).any(|cpu| {
                let data = runtime.graph_data(None, cpu, |_| None);
                data.contributors.available && !data.contributors.rows.is_empty()
            });
            if ready {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let empty = runtime.snapshot();
    assert_eq!(empty.instances, 0);
    assert!(empty.process_cpus.is_empty());
    assert!(
        empty.contributor_identities > 0,
        "host contribution history must run without saved apps"
    );
    assert!(
        ready,
        "no complete host contributor row: {:?}",
        empty.detail_error
    );
    eprintln!(
        "CONTRIBUTORS empty_saved_set=true retained={}",
        empty.contributor_identities
    );
    runtime.configure(false, false);
    runtime.shutdown();
    let until = Instant::now() + Duration::from_secs(8);
    while !runtime.is_stopped() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(runtime.is_stopped());
    let mib = 1024.0 * 1024.0;
    eprintln!(
        "COST cpus={cpus} apps=10 retention=1800 samples={recorded_samples} reserved_mib={:.3} threads={} tables_bytes={}",
        original as f64 / mib,
        before.threads,
        before.bookkeeping_bytes
    );
    for (name, (cpu, private, working, cycles)) in [
        ("disabled_before", baseline),
        ("enabled_closed_graph", enabled),
        ("disabled_after", disabled),
    ] {
        eprintln!(
            "COST {name} one_core_percent={cpu:.4} machine_percent={:.4} private_mib={:.3} working_set_mib={:.3} process_cycles={cycles}",
            cpu / cpus as f64,
            private as f64 / mib,
            working as f64 / mib
        );
    }
    eprintln!(
        "COST enabled_delta_private_mib={:.3} enabled_delta_working_mib={:.3}",
        (enabled.1 as f64 - baseline.1 as f64) / mib,
        (enabled.2 as f64 - baseline.2 as f64) / mib
    );
}
#[test]
fn closing_and_tray_leave_continuous_collection_enabled_without_revision_change() {
    let mut runtime = runtime();
    runtime.configure(true, true);
    let revision = runtime.control.0.lock().unwrap().revision;
    runtime.open(GraphTarget::Process {
        target: target(),
        name: "Game".into(),
    });
    runtime.configure(true, true);
    runtime.configure(true, false);
    runtime.close();
    assert_eq!(runtime.control.0.lock().unwrap().revision, revision);
    assert!(runtime.control.0.lock().unwrap().enabled);
    runtime.set_history_seconds(30);
    assert_eq!(runtime.control.0.lock().unwrap().revision, revision);
    runtime.configure(false, true);
    assert!(!runtime.control.0.lock().unwrap().enabled);
    assert!(runtime.graph.is_none());
    assert!(runtime.worker.is_none());
}
#[test]
fn reopening_graph_reads_preexisting_history_and_reuses_series_capacity() {
    let mut runtime = runtime();
    runtime
        .history
        .write()
        .unwrap()
        .configure(2, 30, &[target()])
        .unwrap();
    runtime.history.write().unwrap().push(
        1.0,
        SystemTime::UNIX_EPOCH,
        &[Some(1.0); 2],
        &[],
        &HashMap::from([(target(), vec![Some(25.0); 2])]),
    );
    runtime.open(GraphTarget::Process {
        target: target(),
        name: "Game".into(),
    });
    assert_eq!(
        runtime.series(Some(&target()), 0).points[0].value,
        Some(25.0)
    );
    runtime.close();
    runtime.configure(true, false);
    assert_eq!(
        runtime.series(Some(&target()), 0).points[0].value,
        Some(25.0)
    );
    let capacity = runtime.series_cache.points.capacity();
    for second in 2..100 {
        runtime.history.write().unwrap().push(
            second as f64,
            SystemTime::UNIX_EPOCH,
            &[],
            &[],
            &HashMap::new(),
        );
        runtime.series(Some(&target()), 0);
    }
    assert_eq!(runtime.series_cache.points.capacity(), capacity);
}
#[test]
fn memory_estimate_has_one_conservative_total_and_keeps_breakdown() {
    let runtime = runtime();
    let estimate = runtime.memory_estimate(20, 10, 1800);
    let summary = estimate.lines().next().unwrap();
    let total = summary
        .strip_prefix("Extra memory: ~")
        .and_then(|value| value.strip_suffix(" MiB"))
        .and_then(|value| value.parse::<f64>().ok())
        .expect("settings need one estimated total, not a range or sum");
    let expected = (history_bytes(20, 10, 1800).unwrap()
        + contributor_bytes(20, 1800).unwrap()
        + os_api::cpu_load::history_buffer_bytes(20).unwrap()
        + ESTIMATED_COLLECTOR_RUNTIME_BYTES
        + 20 * 10 * 24
        + 20 * 16
        + 20 * 2 * 2 * 64 * 1024) as f64
        / (1024.0 * 1024.0);
    assert!((total - expected).abs() <= 0.05);
    assert!(estimate.contains("History:"));
    assert!(estimate.contains("20 logical CPUs · 10 apps"));
    assert!(estimate.contains("0 tracked processes · 0 observed threads"));
}

#[test]
fn estimate_changes_with_app_count_retention_and_current_processes() {
    let runtime = runtime();
    assert_ne!(
        runtime.memory_estimate(20, 10, 1800),
        runtime.memory_estimate(20, 11, 1800)
    );
    assert_ne!(
        runtime.memory_estimate(20, 10, 30),
        runtime.memory_estimate(20, 10, 1800)
    );
    assert!(runtime
        .memory_estimate(20, 10, 1800)
        .contains("20 logical CPUs · 10 apps"));
}
#[test]
fn stale_revision_and_stale_time_are_never_displayed() {
    let mut runtime = runtime();
    runtime.configure(true, true);
    let old = Snapshot {
        revision: runtime.control.0.lock().unwrap().revision,
        at: Instant::now() - Duration::from_secs(5),
        cpus: vec![Some(100.0)],
        ..Snapshot::default()
    };
    *runtime.published.write().unwrap() = Arc::new(old);
    assert!(runtime.snapshot().cpus.is_empty());
}
#[test]
fn preference_save_and_reopen_preserve_classic_monitoring() {
    use crate::app::runtime::AppState;
    let storage = Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {
        s.process_monitoring_enabled = true
    })));
    let mut app = AppState::new_for_test(storage.clone(), 8);
    app.set_cpu_load_enabled(false).unwrap();
    app.wait_for_test_save();
    assert!(!app.cpu_load_enabled());
    assert!(storage.read().unwrap().process_monitoring_enabled);
    let restored =
        serde_json::from_str(&serde_json::to_string(&*storage.read().unwrap()).unwrap()).unwrap();
    let app = AppState::new_for_test(Arc::new(RwLock::new(restored)), 8);
    assert!(!app.cpu_load_enabled());
    assert!(app.is_process_monitoring_enabled());
}
