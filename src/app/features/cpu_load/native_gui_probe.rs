//! Opt-in measurement only: production App, native window, isolated sidecar.
use super::*;
use crate::app::shell::App;
use eframe::egui;
use serde_json::{json, Value};

fn phase(seconds: f64) -> &'static str {
    match seconds as u64 {
        0..=4 => "warm_off",
        5..=24 => "off",
        25..=34 => "warm_on",
        35..=59 => "on_idle",
        60..=139 => "interaction",
        140..=159 => "redraw",
        _ => "drain",
    }
}

struct Probe {
    app: App,
    start: Instant,
    last_sample: Instant,
    last_resource: os_api::cpu_load::ResourceSample,
    last_phase: &'static str,
    resources: Vec<Value>,
    frames: Vec<(&'static str, f64)>,
    logic_ms: Vec<f64>,
    ui_ms: Vec<f64>,
    redraw_spacing: Vec<f64>,
    previous_frame: Option<(&'static str, Instant)>,
    errors: HashSet<String>,
    rows: Vec<(SampleTime, u8, usize)>,
    since: f64,
    max_pending: usize,
    max_lag: f64,
    max_poll_gap: f64,
    last_snapshot: Option<Instant>,
    finalized: bool,
    output: std::path::PathBuf,
}
impl Probe {
    fn finish(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let runtime = &mut self.app.state.cpu_load;
        let snapshot = runtime.snapshot();
        let (mut complete, mut pending, mut lost, mut unknown_cells) = (0, 0, 0, 0);
        if let Ok(history) = runtime.history.read() {
            history.probe_rows(self.since, &mut self.rows);
        }
        let mut same_time_checks = 0;
        for &(time, state, unknown) in &self.rows {
            match state {
                0 => pending += 1,
                1 => {
                    complete += 1;
                    unknown_cells += unknown;
                }
                _ => lost += 1,
            }
            if state == 1 {
                for cpu in 0..snapshot.cpus.len() {
                    let data = runtime.graph_data(None, cpu, |_| Some(time));
                    assert_eq!(data.sample.time, Some(time));
                    assert_eq!(data.contributors.time, Some(time));
                    if data.contributors.available && !data.contributors.partial {
                        let sum: f32 = data.contributors.rows.iter().map(|r| r.load).sum();
                        let total = data.sample.values[cpu].unwrap();
                        assert!(
                            (sum - total).abs() < 0.05,
                            "same-time CPU{cpu}: {sum} vs {total}"
                        );
                        same_time_checks += 1;
                    }
                }
            }
        }
        let summarize = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            let at = |q: f64| {
                values
                    .get(((values.len().saturating_sub(1)) as f64 * q).round() as usize)
                    .copied()
            };
            json!({"count":values.len(),"p50_ms":at(0.5),"p95_ms":at(0.95),"max_ms":values.last()})
        };
        let mut frame_summary = serde_json::Map::new();
        for p in ["off", "on_idle", "interaction", "redraw", "drain"] {
            let mut values = self
                .frames
                .iter()
                .filter(|(key, _)| *key == p)
                .map(|(_, v)| *v)
                .collect();
            frame_summary.insert(p.into(), summarize(&mut values));
        }
        let report = json!({
            "mask":os_api::OS::get_process_affinity(std::process::id()).unwrap(),
            "priority":format!("{:?}",os_api::OS::get_process_priority(std::process::id()).unwrap()),
            "logical_cpus":snapshot.cpus.len(),"instances":snapshot.instances,"threads":snapshot.threads,
            "resources":self.resources,"frames":frame_summary,
            "logic":summarize(&mut self.logic_ms),"ui":summarize(&mut self.ui_ms),
            "redraw_frame_spacing":summarize(&mut self.redraw_spacing),
            "history":{"complete":complete,"pending":pending,"lost":lost,"unknown_system_cells":unknown_cells,
                "max_pending":self.max_pending,"max_original_sample_lag_s":self.max_lag,
                "max_worker_publication_gap_s":self.max_poll_gap,"same_time_checks":same_time_checks},
            "errors":self.errors,"elapsed_s":self.start.elapsed().as_secs_f64(),
            "history_bytes":snapshot.history_bytes,
            "estimated_extra_bytes":runtime.memory_estimate(snapshot.cpus.len(), runtime.source.read().unwrap().groups.iter().map(|g|g.programs.len()).sum(), 1800)
        });
        std::fs::write(&self.output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        runtime.shutdown();
        let until = Instant::now() + Duration::from_secs(8);
        while !runtime.is_stopped() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(runtime.is_stopped(), "trace did not stop");
        eprintln!("CPU19_GUI_PROBE report={} complete={complete} pending={pending} lost={lost} errors={:?}",self.output.display(),self.errors);
        assert!(complete >= 90, "not enough completed original intervals");
        assert_eq!(lost, 0, "permanent history loss after warmup");
        assert_eq!(unknown_cells, 0, "unavailable system CPU samples");
    }
}
impl eframe::App for Probe {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let elapsed = self.start.elapsed().as_secs_f64();
        let current = phase(elapsed);
        self.app.state.cpu_load.preference_override = Some(elapsed >= 25.0);
        let before = Instant::now();
        self.app.logic(ctx, frame);
        if elapsed >= 35.0 {
            self.logic_ms.push(before.elapsed().as_secs_f64() * 1000.0);
        }
        if self.last_sample.elapsed() >= Duration::from_secs(1) && !self.finalized {
            let now = Instant::now();
            let resource = os_api::cpu_load::resource_sample().unwrap();
            let span = now.duration_since(self.last_sample).as_secs_f64();
            if current == self.last_phase {
                self.resources.push(json!({"phase":current,"elapsed_s":elapsed,"span_s":span,
                    "one_cpu_percent":(resource.cpu_ticks-self.last_resource.cpu_ticks) as f64/10_000_000.0/span*100.0,
                    "private_bytes":resource.private_bytes,"working_set":resource.working_set,
                    "graph_open":self.app.state.cpu_load.graph.is_some()}));
            }
            self.last_resource = resource;
            self.last_sample = now;
            self.last_phase = current;
            if elapsed >= 35.0 {
                let runtime = &self.app.state.cpu_load;
                let snapshot = runtime.snapshot();
                if let Some(span) = self
                    .last_snapshot
                    .filter(|at| *at != snapshot.at)
                    .and_then(|at| snapshot.at.checked_duration_since(at))
                {
                    self.max_poll_gap = self.max_poll_gap.max(span.as_secs_f64());
                }
                self.last_snapshot = Some(snapshot.at);
                for error in [&snapshot.error, &snapshot.detail_error]
                    .into_iter()
                    .flatten()
                {
                    self.errors.insert(error.clone());
                }
                if let Ok(history) = runtime.history.try_read() {
                    self.max_pending = self.max_pending.max(history.pending_count());
                    if self.since == 0.0 {
                        self.since = history
                            .latest_finalized()
                            .map(|t| t.seconds)
                            .unwrap_or_default();
                    }
                    if let Some(time) = history.latest_finalized() {
                        self.max_lag = self.max_lag.max(
                            SystemTime::now()
                                .duration_since(time.wall)
                                .unwrap_or_default()
                                .as_secs_f64(),
                        );
                    }
                }
            }
            eprintln!(
                "CPU19_PHASE {current} t={elapsed:.0} graph={}",
                self.app.state.cpu_load.graph.is_some()
            );
        }
        if elapsed >= 165.0 {
            self.finish();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if current == "redraw" {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let elapsed = self.start.elapsed().as_secs_f64();
        let current = phase(elapsed);
        let now = Instant::now();
        if let Some((previous, at)) = self.previous_frame {
            if current == "redraw" && previous == current {
                self.redraw_spacing
                    .push(now.duration_since(at).as_secs_f64() * 1000.0);
            }
        }
        self.previous_frame = Some((current, now));
        if let Some(seconds) = frame.info().cpu_usage {
            self.frames.push((phase(elapsed), seconds as f64 * 1000.0));
        }
        let before = Instant::now();
        self.app.ui(ui, frame);
        if elapsed >= 35.0 {
            self.ui_ms.push(before.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

#[test]
#[ignore = "explicit elevated CPU19 native GUI/CPU/RAM/history probe; 165 seconds, isolated state only"]
fn native_cpu19_gui_cost_and_history() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let directory = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(
        directory.join("cpu19-probe-isolated-state").exists(),
        "refuse real user configuration"
    );
    let messages = crate::app::startup::prepare_background_scheduling_with(
        crate::app::adapters::os::set_current_process_priority,
        crate::app::adapters::os::pin_current_process_to_last_cpu,
    );
    eprintln!("CPU19_SCHEDULING {messages:?}");
    assert_eq!(
        os_api::OS::get_process_affinity(std::process::id()).unwrap(),
        1 << 19
    );
    assert_eq!(
        os_api::OS::get_process_priority(std::process::id()).unwrap(),
        os_api::PriorityClass::BelowNormal
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let options = eframe::NativeOptions {
        viewport: crate::windows_viewport(false),
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_any_thread(true);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "CPU19 performance probe",
        options,
        Box::new(move |cc| {
            let mut app = App::new_without_startup_intent(cc);
            app.state.cpu_load.preference_override = Some(false);
            Ok(Box::new(Probe {
                app,
                start: Instant::now(),
                last_sample: Instant::now(),
                last_resource: os_api::cpu_load::resource_sample().unwrap(),
                last_phase: "warm_off",
                resources: Vec::with_capacity(180),
                frames: Vec::with_capacity(10000),
                logic_ms: Vec::with_capacity(10000),
                ui_ms: Vec::with_capacity(10000),
                redraw_spacing: Vec::with_capacity(10000),
                previous_frame: None,
                errors: HashSet::new(),
                rows: Vec::with_capacity(180),
                since: 0.0,
                max_pending: 0,
                max_lag: 0.0,
                max_poll_gap: 0.0,
                last_snapshot: None,
                finalized: false,
                output: directory.join("report.json"),
            }))
        }),
    )
    .unwrap();
}
