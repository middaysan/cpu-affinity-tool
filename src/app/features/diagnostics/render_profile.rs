//! Bounded, explicit, nonshipping render diagnostics. No I/O on measured threads.
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const QUEUE: usize = 256;
const LIMIT: usize = 64 * 1024 * 1024;
const END_RESERVE: usize = 1024;
const STAGES: usize = Stage::Reconcile as usize + 1;
#[cfg(all(debug_assertions, feature = "debug-mcp"))]
#[path = "debug_performance.rs"]
mod debug_performance;

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Stage {
    GuiLogic,
    GuiUi,
    Header,
    Footer,
    Central,
    CpuModal,
    GraphRead,
    SeriesCopy,
    OverviewCopy,
    GridTile,
    DetailDraw,
    DetailRebuild,
    MiniDraw,
    MiniRebuild,
    Contributors,
    CurrentProcesses,
    FontPreparation,
    CpuSampling,
    CpuPdh,
    Parking,
    ProcessTimes,
    TraceDrain,
    TraceTick,
    Discovery,
    NativeFrame,
    NativePrepare,
    NativeUpdate,
    Tessellate,
    Paint,
    Reconcile,
}
const NAMES: [&str; STAGES] = [
    "gui_logic",
    "gui_ui",
    "header",
    "footer",
    "central",
    "cpu_modal",
    "graph_read",
    "series_copy",
    "overview_copy",
    "cpu_tile",
    "detail_draw",
    "detail_rebuild",
    "mini_draw",
    "mini_rebuild",
    "contributors",
    "current_processes",
    "font_preparation",
    "cpu_sampling",
    "pdh",
    "parking",
    "process_times",
    "trace_drain",
    "trace_tick",
    "discovery",
    "native_frame",
    "native_prepare",
    "native_update",
    "tessellate",
    "paint",
    "reconcile",
];
#[derive(Clone, Copy, Default, Serialize)]
struct Stat {
    count: u64,
    wall_us: u64,
    max_us: u64,
    cycles: u64,
    unknown_cycles: u64,
}
impl Stat {
    fn add(&mut self, wall: u64, cycles: Option<u64>) {
        self.count = self.count.saturating_add(1);
        self.wall_us = self.wall_us.saturating_add(wall);
        self.max_us = self.max_us.max(wall);
        if let Some(cycles) = cycles {
            self.cycles = self.cycles.saturating_add(cycles);
        } else {
            self.unknown_cycles = self.unknown_cycles.saturating_add(1);
        }
    }
}
#[derive(Default)]
struct Local {
    frame: Option<u64>,
    stats: [Stat; STAGES],
}
impl Local {
    fn begin(&mut self, id: u64) -> Option<FrameData> {
        if self.frame == Some(id) {
            return None;
        }
        let old = self.take();
        self.frame = Some(id);
        old
    }
    fn take(&mut self) -> Option<FrameData> {
        self.frame.take().map(|id| FrameData {
            id,
            stats: std::mem::take(&mut self.stats),
        })
    }
}
thread_local! { static LOCAL: RefCell<Local> = RefCell::new(Local::default()); }
struct FrameData {
    id: u64,
    stats: [Stat; STAGES],
}
#[derive(Default, Serialize)]
pub(crate) struct View {
    pub mode: &'static str,
    pub selected_cpu: Option<usize>,
    pub all_cpus: bool,
    pub input_events: usize,
    pub pass: usize,
}
#[allow(clippy::large_enum_variant)] // Fixed queue storage; no per-frame Box allocation.
enum Event {
    Frame {
        at_us: u64,
        data: FrameData,
        view: View,
        previous_backend_ms: Option<f32>,
        rendered: bool,
    },
    Span {
        at_us: u64,
        stage: Stage,
        wall_us: u64,
        cycles: Option<u64>,
    },
    Note {
        at_us: u64,
        code: &'static str,
    },
    History {
        at_us: u64,
        seconds: f64,
        state: &'static str,
        known: usize,
        total: usize,
        pending: usize,
        gap_reason: Option<&'static str>,
    },
    Telemetry {
        at_us: u64,
        instances: usize,
        threads: usize,
        history_bytes: usize,
        tracking: bool,
        error: &'static str,
    },
    Renderer {
        at_us: u64,
        name: String,
    },
}
struct Shared {
    tx: mpsc::SyncSender<Event>,
    start: Instant,
    enabled: AtomicBool,
    stop: AtomicBool,
    dropped: AtomicU64,
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    inspection: std::sync::Mutex<debug_performance::PerformanceStore>,
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    inspection_cpus: usize,
}
impl Shared {
    fn new(tx: mpsc::SyncSender<Event>) -> Self {
        Self {
            tx,
            start: Instant::now(),
            enabled: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            #[cfg(all(debug_assertions, feature = "debug-mcp"))]
            inspection: std::sync::Mutex::new(debug_performance::PerformanceStore::new()),
            #[cfg(all(debug_assertions, feature = "debug-mcp"))]
            inspection_cpus: os_api::cpu_load::logical_cpu_count(),
        }
    }
    fn now(&self) -> u64 {
        self.start.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
    fn send(&self, event: Event) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.enabled.store(false, Ordering::Relaxed);
            }
        }
    }
}
static ACTIVE: OnceLock<Arc<Shared>> = OnceLock::new();
fn active() -> Option<&'static Shared> {
    ACTIVE
        .get()
        .filter(|s| s.enabled.load(Ordering::Relaxed))
        .map(Arc::as_ref)
}
#[cfg(all(debug_assertions, feature = "debug-mcp"))]
pub(crate) fn performance(seconds: u32) -> Result<Value, String> {
    if !(1..=60).contains(&seconds) {
        return Err("Use a performance window of 1..60 seconds".into());
    }
    let Some(shared) = ACTIVE.get() else {
        return Ok(json!({"available":false,"reason":"profiling_not_started"}));
    };
    let store = shared
        .inspection
        .try_lock()
        .map_err(|_| "Profiler snapshot busy; retry")?;
    let mut value = store.snapshot(shared.now(), seconds, shared.inspection_cpus);
    value["active"] = json!(shared.enabled.load(Ordering::Relaxed));
    value["dropped_records"] = json!(shared.dropped.load(Ordering::Relaxed));
    Ok(value)
}
pub(crate) struct Span {
    stage: Stage,
    start: Instant,
    clock: Option<os_api::render_profile::ThreadClock>,
    frame: Option<u64>,
}
pub(crate) fn span(stage: Stage) -> Option<Span> {
    active()?;
    Some(Span {
        stage,
        start: Instant::now(),
        clock: os_api::render_profile::thread_clock(),
        frame: LOCAL.with_borrow(|l| l.frame),
    })
}
impl Drop for Span {
    fn drop(&mut self) {
        let Some(shared) = active() else {
            return;
        };
        let clock = os_api::render_profile::thread_clock();
        let cycles = self
            .clock
            .zip(clock)
            .filter(|(a, b)| a.thread == b.thread)
            .and_then(|(a, b)| b.cycles.checked_sub(a.cycles));
        let wall_us = self.start.elapsed().as_micros().min(u64::MAX as u128) as u64;
        if self.frame.is_some()
            && LOCAL.with_borrow_mut(|l| {
                if l.frame != self.frame {
                    return false;
                }
                l.stats[self.stage as usize].add(wall_us, cycles);
                true
            })
        {
            return;
        }
        shared.send(Event::Span {
            at_us: shared.now(),
            stage: self.stage,
            wall_us,
            cycles,
        });
    }
}
pub(crate) fn begin_logic(id: u64) {
    let Some(shared) = active() else {
        return;
    };
    if let Some(data) = LOCAL.with_borrow_mut(|l| l.begin(id)) {
        shared.send(Event::Frame {
            at_us: shared.now(),
            data,
            view: View::default(),
            previous_backend_ms: None,
            rendered: false,
        });
    }
}
pub(crate) struct FrameEnd {
    view: Option<View>,
    backend: Option<f32>,
}
pub(crate) fn frame_end(view: View, backend: Option<f32>) -> FrameEnd {
    FrameEnd {
        view: Some(view),
        backend,
    }
}
impl Drop for FrameEnd {
    fn drop(&mut self) {
        let Some(shared) = active() else {
            return;
        };
        if let Some(data) = LOCAL.with_borrow_mut(Local::take) {
            shared.send(Event::Frame {
                at_us: shared.now(),
                data,
                view: self.view.take().unwrap_or_default(),
                previous_backend_ms: self.backend.map(|s| s * 1000.0),
                rendered: true,
            });
        }
    }
}
pub(crate) fn note(code: &'static str) {
    if let Some(s) = active() {
        s.send(Event::Note {
            at_us: s.now(),
            code,
        });
    }
}
pub(crate) fn error_code(error: Option<&str>) -> &'static str {
    match error {
        None => "none",
        Some(e) if e.contains("Waiting for tracked") => "tracking_busy",
        Some(e) if e.contains("changed") => "membership_changed",
        Some(e) if e.contains("lost") => "trace_loss",
        Some(e) if e.contains("schema") || e.contains("version") => "event_schema",
        Some(e) if e.contains("order") => "event_order",
        Some(e) if e.contains("pool") || e.contains("overflow") => "interval_pool",
        Some(e) if e.contains("lock") => "lock_failure",
        Some(_) => "other",
    }
}
pub(crate) fn interval(
    seconds: f64,
    state: &'static str,
    known: usize,
    total: usize,
    pending: usize,
    gap_reason: Option<&'static str>,
) {
    if let Some(s) = active() {
        s.send(Event::History {
            at_us: s.now(),
            seconds,
            state,
            known,
            total,
            pending,
            gap_reason,
        });
    }
}
pub(crate) fn telemetry(
    instances: usize,
    threads: usize,
    history_bytes: usize,
    tracking: bool,
    error: &'static str,
) {
    if let Some(s) = active() {
        s.send(Event::Telemetry {
            at_us: s.now(),
            instances,
            threads,
            history_bytes,
            tracking,
            error,
        });
    }
}
pub(crate) fn renderer(name: String) {
    if let Some(s) = active() {
        s.send(Event::Renderer {
            at_us: s.now(),
            name,
        });
    }
}

struct Sink<W> {
    writer: W,
    bytes: usize,
    limit: usize,
    scratch: Vec<u8>,
}
impl<W: Write> Sink<W> {
    fn new(writer: W, limit: usize) -> Self {
        Self {
            writer,
            bytes: 0,
            limit,
            scratch: Vec::with_capacity(4096),
        }
    }
    fn encode(&mut self, value: &Value) -> std::io::Result<()> {
        self.scratch.clear();
        serde_json::to_writer(&mut self.scratch, value)?;
        self.scratch.push(b'\n');
        Ok(())
    }
    fn line(&mut self, value: &Value) -> std::io::Result<bool> {
        self.encode(value)?;
        if self
            .bytes
            .saturating_add(self.scratch.len())
            .saturating_add(END_RESERVE)
            > self.limit
        {
            return Ok(false);
        }
        self.writer.write_all(&self.scratch)?;
        self.bytes += self.scratch.len();
        Ok(true)
    }
    fn end(&mut self, value: &Value) -> std::io::Result<()> {
        self.encode(value)?;
        if self.bytes.saturating_add(self.scratch.len()) > self.limit {
            return Err(std::io::Error::other(
                "completion marker exceeds log budget",
            ));
        }
        self.writer.write_all(&self.scratch)?;
        self.bytes += self.scratch.len();
        self.writer.flush()
    }
}
fn event_json(event: Event) -> Value {
    match event {
        Event::Frame {
            at_us,
            data,
            view,
            previous_backend_ms,
            rendered,
        } => {
            let stages: serde_json::Map<_, _> = data
                .stats
                .iter()
                .enumerate()
                .filter(|(_, s)| s.count > 0)
                .map(|(i, s)| (NAMES[i].into(), json!(s)))
                .collect();
            json!({"kind":"frame","at_us":at_us,"frame":data.id,"rendered":rendered,"view":view,"previous_frame_backend_ms":previous_backend_ms,"stages":stages})
        }
        Event::Span {
            at_us,
            stage,
            wall_us,
            cycles,
        } => {
            json!({"kind":"span","at_us":at_us,"stage":NAMES[stage as usize],"wall_us":wall_us,"cycles":cycles})
        }
        Event::Note { at_us, code } => json!({"kind":"note","at_us":at_us,"code":code}),
        Event::History {
            at_us,
            seconds,
            state,
            known,
            total,
            pending,
            gap_reason,
        } => {
            json!({"kind":"history","at_us":at_us,"original_seconds":seconds,"state":state,"system_known_cpus":known,"cpus":total,"pending":pending,"gap_reason":gap_reason})
        }
        Event::Telemetry {
            at_us,
            instances,
            threads,
            history_bytes,
            tracking,
            error,
        } => {
            json!({"kind":"telemetry","at_us":at_us,"instances":instances,"observed_threads":threads,"history_bytes":history_bytes,"tracking_available":tracking,"error":error})
        }
        Event::Renderer { at_us, name } => {
            json!({"kind":"renderer","at_us":at_us,"description":name})
        }
    }
}
pub(crate) struct Session {
    shared: Arc<Shared>,
    done: mpsc::Receiver<std::io::Result<()>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.shared.enabled.store(false, Ordering::Relaxed);
        self.shared.stop.store(true, Ordering::Release);
        match self.done.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            result => eprintln!("Render profile completion not confirmed: {result:?}"),
        }
    }
}
fn profile_duration(seconds: Option<&str>) -> Duration {
    Duration::from_secs(
        seconds
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(10, 3600),
    )
}

pub(crate) fn start_from_env() -> Result<Option<Session>, String> {
    let Some(path) = std::env::var_os("CPU_AFFINITY_RENDER_LOG") else {
        return Ok(None);
    };
    let duration = profile_duration(std::env::var("CPU_AFFINITY_RENDER_SECONDS").ok().as_deref());
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Create render log: {e}"))?;
    let (tx, rx) = mpsc::sync_channel(QUEUE);
    let shared = Arc::new(Shared::new(tx));
    ACTIVE
        .set(shared.clone())
        .map_err(|_| "Render profiling already initialized")?;
    let (done_tx, done) = mpsc::sync_channel(1);
    let worker = shared.clone();
    std::thread::Builder::new()
        .name("render-log".into())
        .spawn(move || {
            let result = write_session(
                BufWriter::with_capacity(64 * 1024, file),
                &worker,
                rx,
                duration,
            );
            worker.enabled.store(false, Ordering::Relaxed);
            let _ = done_tx.send(result);
        })
        .map_err(|e| format!("Start render logger: {e}"))?;
    Ok(Some(Session { shared, done }))
}
fn write_session<W: Write>(
    writer: W,
    shared: &Shared,
    rx: mpsc::Receiver<Event>,
    duration: Duration,
) -> std::io::Result<()> {
    let mut sink = Sink::new(writer, LIMIT);
    let qpc_before = os_api::render_profile::qpc_clock();
    let clock_us = shared.now();
    let qpc_after = os_api::render_profile::qpc_clock();
    sink.line(&json!({"kind":"session_start","schema":1,"version":env!("CARGO_PKG_VERSION"),"unix_ms":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis(),"logical_cpus":os_api::cpu_load::logical_cpu_count(),"affinity_mask":os_api::OS::get_process_affinity(std::process::id()).ok(),"priority":format!("{:?}",os_api::OS::get_process_priority(std::process::id()).ok()),"limit_bytes":LIMIT,"limit_seconds":duration.as_secs(),"queue_capacity":QUEUE,"timing":"inclusive wall_us; unscaled scheduled thread cycles; backend timing belongs to previous frame; no GPU/input-to-display timing"}))?;
    sink.line(&json!({"kind":"clock_anchor","clock_us":clock_us,"qpc_before":qpc_before,"qpc_after":qpc_after}))?;
    let mut resource = os_api::cpu_load::resource_sample().ok();
    let mut sampled = Instant::now();
    let mut writer_clock = os_api::render_profile::thread_clock();
    let mut stopping = None;
    let reason = loop {
        if shared.start.elapsed() >= duration {
            break "time_limit";
        }
        if shared.stop.load(Ordering::Acquire) {
            let deadline = stopping.get_or_insert_with(|| Instant::now() + Duration::from_secs(1));
            if Instant::now() >= *deadline {
                break "shutdown_drain_timeout";
            }
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                #[cfg(all(debug_assertions, feature = "debug-mcp"))]
                if let Ok(mut store) = shared.inspection.lock() {
                    store.observe(&event);
                }
                if !sink.line(&event_json(event))? {
                    break "size_limit";
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) if stopping.is_some() => break "app_exit",
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break "disconnected",
        }
        if sampled.elapsed() >= Duration::from_secs(1) {
            let now = Instant::now();
            let span = now.duration_since(sampled).as_secs_f64();
            let after = os_api::cpu_load::resource_sample().ok();
            let next_clock = os_api::render_profile::thread_clock();
            let value = match (&resource, &after) {
                (Some(a), Some(b)) => {
                    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
                    if let Ok(mut store) = shared.inspection.lock() {
                        store.resource(debug_performance::ResourceRecord {
                            at_us: shared.now(),
                            span_s: span,
                            one_cpu_percent: b.cpu_ticks.saturating_sub(a.cpu_ticks) as f64
                                / 10000000.0
                                / span
                                * 100.0,
                            private_bytes: b.private_bytes,
                            working_bytes: b.working_set,
                        });
                    }
                    json!({"kind":"resource","at_us":shared.now(),"span_s":span,"one_cpu_percent":b.cpu_ticks.saturating_sub(a.cpu_ticks) as f64/10000000.0/span*100.0,"private_bytes":b.private_bytes,"working_set_bytes":b.working_set,"process_cycles":b.cycles.saturating_sub(a.cycles),"writer_cycles":writer_clock.zip(next_clock).and_then(|(a,b)|b.cycles.checked_sub(a.cycles)),"dropped_records":shared.dropped.load(Ordering::Relaxed),"log_bytes":sink.bytes})
                }
                _ => {
                    json!({"kind":"note","at_us":shared.now(),"code":"resource_sample_unavailable"})
                }
            };
            if !sink.line(&value)? {
                break "size_limit";
            }
            sink.writer.flush()?;
            resource = after;
            sampled = now;
            writer_clock = next_clock;
        }
    };
    shared.enabled.store(false, Ordering::Relaxed);
    sink.end(&json!({"kind":"session_end","at_us":shared.now(),"reason":reason,"dropped_records":shared.dropped.load(Ordering::Relaxed),"queued_records":rx.try_iter().count(),"bytes_before_end":sink.bytes}))
}

#[cfg(test)]
#[path = "render_profile_tests.rs"]
mod tests;
