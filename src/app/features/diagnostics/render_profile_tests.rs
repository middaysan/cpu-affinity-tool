use super::*;

#[test]
fn stats_keep_total_max_count_and_unknown_cycles_without_wraparound() {
    let mut stat = Stat::default();
    stat.add(10, Some(20));
    stat.add(30, None);
    assert_eq!(
        (
            stat.count,
            stat.wall_us,
            stat.max_us,
            stat.cycles,
            stat.unknown_cycles
        ),
        (2, 40, 30, 20, 1)
    );
    stat.wall_us = u64::MAX;
    stat.add(1, Some(0));
    assert_eq!(stat.wall_us, u64::MAX);
}

#[test]
fn frame_reset_never_moves_old_measurements_to_a_new_frame() {
    let mut local = Local::default();
    assert!(local.begin(10).is_none());
    local.stats[Stage::GuiUi as usize].add(20, Some(30));
    assert!(local.begin(10).is_none());
    let abandoned = local.begin(11).unwrap();
    assert_eq!(abandoned.id, 10);
    assert_eq!(abandoned.stats[Stage::GuiUi as usize].wall_us, 20);
    assert!(local.stats.iter().all(|s| s.count == 0));
    let complete = local.take().unwrap();
    assert_eq!(complete.id, 11);
    assert!(local.take().is_none());
    assert!(local.begin(11).is_none()); // Another layout pass of this same frame.
    assert!(local.take().unwrap().stats.iter().all(|s| s.count == 0));
}

#[test]
fn bounded_queue_never_blocks_and_reports_dropped_records() {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let shared = Shared::new(tx);
    shared.send(Event::Note {
        at_us: 1,
        code: "test",
    });
    shared.send(Event::Note {
        at_us: 2,
        code: "test",
    });
    assert_eq!(shared.dropped.load(Ordering::Relaxed), 1);
    assert!(rx.try_recv().is_ok());
    drop(rx);
    shared.send(Event::Note {
        at_us: 3,
        code: "test",
    });
    assert!(!shared.enabled.load(Ordering::Relaxed));
}

#[test]
fn log_budget_keeps_valid_json_lines_and_reserves_a_completion_marker() {
    let mut sink = Sink::new(Vec::new(), 2048);
    let mut count = 0;
    while sink
        .line(&serde_json::json!({"kind":"frame","payload":"x".repeat(200)}))
        .unwrap()
    {
        count += 1;
    }
    assert!(count > 0);
    sink.end(&serde_json::json!({"kind":"session_end","reason":"size_limit"}))
        .unwrap();
    assert!(sink.bytes <= 2048);
    let text = String::from_utf8(sink.writer).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(rows.last().unwrap()["kind"], "session_end");
    assert_eq!(rows.len(), count + 1);
}

#[test]
fn slow_disk_errors_are_not_reported_as_a_complete_log() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("fixture"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(Sink::new(Broken, 2048)
        .line(&serde_json::json!({"kind":"frame"}))
        .is_err());
}

#[test]
fn disabled_capture_does_not_read_clocks_or_create_a_scope() {
    assert!(span(Stage::GuiUi).is_none());
}

#[test]
fn diagnostic_categories_expose_membership_waits_without_logging_raw_details() {
    assert_eq!(error_code(None), "none");
    assert_eq!(
        error_code(Some("Waiting for tracked processes")),
        "tracking_busy"
    );
    assert_eq!(
        error_code(Some(
            "Tracked processes changed; waiting for the next interval"
        )),
        "membership_changed"
    );
    assert_eq!(error_code(Some("ETW lost 4 events")), "trace_loss");
    assert_eq!(
        error_code(Some("unclassified private fixture text")),
        "other"
    );
}

#[test]
fn render_log_records_frames_background_work_resources_and_clean_shutdown() {
    use std::os::windows::process::CommandExt;
    let dir = std::env::temp_dir().join(format!(
        "cpu-render-log-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("smoke.jsonl");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "app::features::diagnostics::render_profile::tests::render_log_smoke_child",
            "--nocapture",
        ])
        .env("CPU_AFFINITY_RENDER_LOG", &path)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.first().unwrap()["kind"], "session_start");
    assert_eq!(rows.last().unwrap()["kind"], "session_end");
    assert_eq!(rows.last().unwrap()["dropped_records"], 0);
    assert!(rows
        .iter()
        .any(|r| r["kind"] == "resource" && r["one_cpu_percent"].is_number()));
    assert!(rows
        .iter()
        .any(|r| r["kind"] == "span" && r["stage"] == "discovery"));
    let frames: Vec<_> = rows.iter().filter(|r| r["kind"] == "frame").collect();
    assert_eq!(frames.len(), 20);
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(frame["frame"], index);
        assert_eq!(frame["stages"]["gui_ui"]["count"], 1);
        assert_eq!(frame["previous_frame_backend_ms"], 2.0);
        assert!(frame["stages"]["gui_ui"]["cycles"].as_u64().is_some());
        assert_eq!(frame["stages"]["gui_ui"]["unknown_cycles"], 0);
    }
    assert!(!text.contains("bin_path") && !text.contains("username"));
}

#[test]
#[ignore = "isolated logger subprocess invoked by its parent; no GUI or scheduler trace"]
fn render_log_smoke_child() {
    let session = start_from_env().unwrap().unwrap();
    for id in 0..20 {
        begin_logic(id);
        let _frame = frame_end(
            View {
                mode: "smoke",
                ..Default::default()
            },
            Some(0.002),
        );
        let _scope = span(Stage::GuiUi);
        std::hint::black_box((0..1000).sum::<u64>());
    }
    std::thread::spawn(|| {
        let _scope = span(Stage::Discovery);
        std::thread::sleep(Duration::from_millis(2));
    })
    .join()
    .unwrap();
    interval(1.0, "pending", 0, 20, 1, None);
    interval(1.0, "complete", 20, 20, 0, None);
    std::thread::sleep(Duration::from_millis(1100));
    drop(session);
}
