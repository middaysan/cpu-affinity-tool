use super::*;
#[test]
fn performance_window_reuses_stages_weights_cpu_and_does_not_sum_inclusive_children() {
    let mut store = PerformanceStore::new();
    let mut stats = [Stat::default(); STAGES];
    stats[Stage::GuiUi as usize].add(2000, Some(5));
    stats[Stage::Header as usize].add(1800, Some(4));
    store.frame(
        2_000_000,
        7,
        &stats,
        &View {
            mode: "cpu_graph",
            ..Default::default()
        },
        Some(50.0),
    );
    store.frame(
        2_000_100,
        7,
        &stats,
        &View {
            mode: "cpu_graph",
            pass: 1,
            ..Default::default()
        },
        Some(50.0),
    );
    store.resource(ResourceRecord {
        at_us: 2_500_000,
        span_s: 1.0,
        one_cpu_percent: 20.0,
        private_bytes: 100,
        working_bytes: 80,
    });
    store.resource(ResourceRecord {
        at_us: 4_500_000,
        span_s: 2.0,
        one_cpu_percent: 5.0,
        private_bytes: 120,
        working_bytes: 90,
    });
    let value = store.snapshot(5_000_000, 5, 20);
    assert_eq!(value["frames"]["count"], 1);
    assert_eq!(
        value["frames"]["max_gui_ms"], 4.0,
        "children are already included in gui_ui"
    );
    assert_eq!(value["frames"]["slowest"][0]["passes"], 2);
    assert_eq!(value["resource"]["cpu_one_logical_percent_mean"], 10.0);
    assert_eq!(value["resource"]["cpu_whole_machine_percent_mean"], 0.5);
    assert_eq!(value["frames"]["previous_backend_ms_p95"], 50.0);
    assert_eq!(store.snapshot(70_000_000, 5, 20)["frames"]["count"], 0);
}
#[test]
fn frame_storage_is_bounded_and_reports_eviction_in_the_requested_window() {
    let mut store = PerformanceStore::new();
    let stats = [Stat::default(); STAGES];
    for id in 0..(FRAME_CAPACITY + 2) {
        store.frame(
            1_000_000 + id as u64,
            id as u64,
            &stats,
            &View::default(),
            None,
        );
    }
    assert_eq!(store.frames.len(), FRAME_CAPACITY);
    let value = store.snapshot(2_000_000, 5, 20);
    assert_eq!(value["frames"]["evicted_in_window"], true);
    assert!(value["resource"]["cpu_one_logical_percent_mean"].is_null());
}
