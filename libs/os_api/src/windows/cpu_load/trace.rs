use std::collections::BTreeMap;
use std::mem::{offset_of, size_of};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{ERROR_CTX_CLOSE_PENDING, ERROR_SUCCESS};
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::Diagnostics::Etw::{
    CONTROLTRACE_HANDLE, CloseTrace, ControlTraceW, EVENT_HEADER_FLAG_PROCESSOR_INDEX,
    EVENT_RECORD, EVENT_TRACE_CONTROL_QUERY, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_FLAG_CSWITCH,
    EVENT_TRACE_FLAG_NO_SYSCONFIG, EVENT_TRACE_FLAG_PROCESS, EVENT_TRACE_FLAG_THREAD,
    EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    EVENT_TRACE_SYSTEM_LOGGER_MODE, OpenTraceW, PROCESS_TRACE_MODE_EVENT_RECORD,
    PROCESS_TRACE_MODE_RAW_TIMESTAMP, PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE,
    PROPERTY_DATA_DESCRIPTOR, ProcessTrace, StartTraceW, TdhGetProperty, TdhGetPropertySize,
    ThreadGuid, WNODE_FLAG_TRACED_GUID,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::core::{GUID, PCWSTR, PWSTR, w};

use super::accounting::{SharedAccounting, event_cpu_index, scheduler_pair};
use super::{PinnedProcess, coordinates, status};

// EVENT_TRACE_PROPERTIES requires its strings in the same allocation. This
// uses generated SDK types and a naturally aligned, fixed-size owned buffer.
#[repr(C)]
struct Properties {
    value: EVENT_TRACE_PROPERTIES,
    name: [u16; 128],
}

impl Properties {
    fn new(name: &[u16], guid: GUID) -> Self {
        let mut result = Self {
            value: EVENT_TRACE_PROPERTIES::default(),
            name: [0; 128],
        };
        result.value.Wnode.BufferSize = size_of::<Self>() as u32;
        result.value.Wnode.Guid = guid;
        result.value.Wnode.ClientContext = 1; // Documented QPC clock.
        result.value.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        result.value.LoggerNameOffset = offset_of!(Self, name) as u32;
        result.name[..name.len()].copy_from_slice(name);
        result
    }
}

struct OwnedSession {
    handle: CONTROLTRACE_HANDLE,
    guid: GUID,
    name: Vec<u16>,
}

impl OwnedSession {
    fn start(cpus: usize, contributors: bool) -> Result<Self, String> {
        let guid = unsafe { CoCreateGuid() }
            .map_err(|error| format!("Create CPU trace identity: {error}"))?;
        let name: Vec<_> = format!("CpuAffinityTool.CpuLoad.{guid:?}")
            .encode_utf16()
            .chain([0])
            .collect();
        if name.len() > 128 {
            return Err("CPU trace name exceeds the bounded session buffer".into());
        }
        let mut properties = Properties::new(&name, guid);
        properties.value.LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE;
        properties.value.EnableFlags =
            EVENT_TRACE_FLAG_CSWITCH | EVENT_TRACE_FLAG_THREAD | EVENT_TRACE_FLAG_NO_SYSCONFIG;
        if contributors {
            properties.value.EnableFlags |= EVENT_TRACE_FLAG_PROCESS;
        }
        properties.value.BufferSize = 64; // KiB, no file/log path or stack walk.
        properties.value.MinimumBuffers = (cpus as u32 * 2).max(4);
        properties.value.MaximumBuffers = properties.value.MinimumBuffers * 2;
        properties.value.FlushTimer = 1;
        let mut handle = CONTROLTRACE_HANDLE::default();
        // Never use NT Kernel Logger, SystemTraceControlGuid, an existing name,
        // or enumerate/stop other sessions when the logger limit is reached.
        status(
            "StartTraceW (detailed CPU monitoring requires administrator access)",
            unsafe { StartTraceW(&mut handle, PCWSTR(name.as_ptr()), &mut properties.value) }.0,
        )?;
        Ok(Self { handle, guid, name })
    }

    fn query(&self) -> Result<Properties, String> {
        let mut properties = Properties::new(&self.name, self.guid);
        status(
            "Query own CPU trace",
            unsafe {
                ControlTraceW(
                    self.handle,
                    PCWSTR::null(),
                    &mut properties.value,
                    EVENT_TRACE_CONTROL_QUERY,
                )
            }
            .0,
        )?;
        if properties.value.Wnode.Guid != self.guid {
            return Err("CPU trace identity changed; refusing to control another session".into());
        }
        Ok(properties)
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.handle.Value == 0 {
            return Ok(());
        }
        let mut properties = self.query()?;
        status(
            "Stop own CPU trace",
            unsafe {
                ControlTraceW(
                    self.handle,
                    PCWSTR::null(),
                    &mut properties.value,
                    EVENT_TRACE_CONTROL_STOP,
                )
            }
            .0,
        )?;
        self.handle.Value = 0;
        Ok(())
    }
}

impl Drop for OwnedSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct ConsumerContext {
    accounting: Mutex<SharedAccounting>,
    stopping: AtomicBool,
    cpus: usize,
}

impl ConsumerContext {
    fn fail(&self, message: impl Into<String>) {
        if let Ok(mut accounting) = self.accounting.lock() {
            accounting.fail(message);
        }
    }

    fn event(&self, event: &EVENT_RECORD) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        const PROCESS_GUID: GUID = GUID::from_u128(0x3d6fa8d0_fe05_11d0_9dda_00c04fd7ba7c);
        if event.EventHeader.ProviderId == PROCESS_GUID {
            let opcode = event.EventHeader.EventDescriptor.Opcode;
            if !matches!(opcode, 1..=3) {
                return;
            }
            let pid = match process_property(event, w!("ProcessId"), &mut [0; 4]) {
                Ok(bytes) if bytes.len() == 4 => u32::from_le_bytes(bytes.try_into().unwrap()),
                _ => {
                    self.fail("ETW process identity could not be decoded");
                    return;
                }
            };
            let mut name = [0; 512];
            let name = if opcode != 2 {
                match process_property(event, w!("ImageFileName"), &mut name) {
                    Ok(bytes) => {
                        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                        let text = String::from_utf8_lossy(&bytes[..end]);
                        let text = text.rsplit(['\\', '/']).next().unwrap_or_default();
                        if text.is_empty() {
                            "Name unavailable".into()
                        } else {
                            text.chars()
                                .filter(|c| !c.is_control())
                                .take(255)
                                .collect::<String>()
                                .into()
                        }
                    }
                    Err(_) => "Name unavailable".into(),
                }
            } else {
                "".into()
            };
            if let Ok(mut accounting) = self.accounting.lock()
                && !accounting.failed()
            {
                if opcode == 2 {
                    accounting.process_end(pid, event.EventHeader.TimeStamp);
                } else {
                    accounting.process_start(pid, name, event.EventHeader.TimeStamp, opcode == 3);
                }
            }
            return;
        }
        if event.EventHeader.ProviderId != ThreadGuid {
            return;
        }
        let opcode = event.EventHeader.EventDescriptor.Opcode;
        if !matches!(opcode, 1..=4 | 36) {
            return;
        }
        let result = if opcode == 36 {
            event_pair(event, w!("NewThreadId"), w!("OldThreadId"))
        } else {
            event_pair(event, w!("ProcessId"), w!("TThreadId"))
        };
        let (first, second) = match result {
            Ok(pair) => pair,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let timestamp = event.EventHeader.TimeStamp;
        let Ok(mut accounting) = self.accounting.lock() else {
            return;
        };
        if accounting.failed() {
            return;
        }
        match opcode {
            1 | 3 => accounting.thread_start(second, first, timestamp),
            2 | 4 => accounting.thread_end(second, timestamp),
            36 => {
                // PROCESSOR_INDEX is the systemwide index. Do not truncate it
                // to the legacy byte on large processor-group systems.
                let cpu = unsafe {
                    event_cpu_index(
                        u32::from(event.EventHeader.Flags) & EVENT_HEADER_FLAG_PROCESSOR_INDEX != 0,
                        event.BufferContext.Anonymous.ProcessorIndex,
                        event.BufferContext.Anonymous.Anonymous.ProcessorNumber,
                        self.cpus,
                    )
                };
                if let Some(cpu) = cpu {
                    accounting.switch(cpu, second, first, timestamp);
                } else {
                    accounting.fail(
                        "ETW logical CPU index is unavailable or outside the captured inventory",
                    );
                }
            }
            _ => unreachable!(),
        }
    }
}

// Metadata decoding happens only on process lifecycle events, never CSwitch.
// Only bounded PID/name fields are requested; SID and command line are ignored.
fn process_property<'a>(
    event: &EVENT_RECORD,
    name: PCWSTR,
    buffer: &'a mut [u8],
) -> Result<&'a [u8], String> {
    let descriptor = PROPERTY_DATA_DESCRIPTOR {
        PropertyName: name.0 as u64,
        ArrayIndex: u32::MAX,
        Reserved: 0,
    };
    let mut size = 0;
    let result = unsafe { TdhGetPropertySize(event, None, &[descriptor], &mut size) };
    if result != 0 || size as usize > buffer.len() {
        return Err("ETW process property is unavailable or oversized".into());
    }
    let buffer = &mut buffer[..size as usize];
    if unsafe { TdhGetProperty(event, None, &[descriptor], buffer) } != 0 {
        return Err("ETW process property decoding failed".into());
    }
    Ok(buffer)
}

// Documented Thread_V2/CSwitch first fields, independent of pointer width:
// https://learn.microsoft.com/windows/win32/etw/thread-v2-typegroup1
// https://learn.microsoft.com/windows/win32/etw/cswitch
// Microsoft PerfView confirms the appended-field schemas handled by
// scheduler_pair. Unknown schema versions still require successful TDH decoding.
fn event_pair(event: &EVENT_RECORD, first: PCWSTR, second: PCWSTR) -> Result<(u32, u32), String> {
    let bytes = if event.UserData.is_null() {
        &[]
    } else {
        unsafe {
            std::slice::from_raw_parts(
                event.UserData.cast::<u8>(),
                usize::from(event.UserDataLength),
            )
        }
    };
    if let Some(pair) = scheduler_pair(
        event.EventHeader.EventDescriptor.Opcode,
        event.EventHeader.EventDescriptor.Version,
        bytes,
    ) {
        return pair;
    }
    let property = |name: PCWSTR| -> Result<u32, String> {
        let descriptor = PROPERTY_DATA_DESCRIPTOR {
            PropertyName: name.0 as u64,
            ArrayIndex: u32::MAX,
            Reserved: 0,
        };
        let mut value = [0u8; 4];
        let result = unsafe { TdhGetProperty(event, None, &[descriptor], &mut value) };
        if result != 0 {
            // The names here are fixed literals supplied above, never payloads.
            let property = unsafe { name.to_string() }.unwrap_or_default();
            return Err(format!(
                "Decode ETW scheduler opcode {} version {} property {property} ({} bytes) failed (Windows status 0x{result:08X})",
                event.EventHeader.EventDescriptor.Opcode,
                event.EventHeader.EventDescriptor.Version,
                event.UserDataLength,
            ));
        }
        Ok(u32::from_le_bytes(value))
    };
    Ok((property(first)?, property(second)?))
}

unsafe extern "system" fn event_callback(event: *mut EVENT_RECORD) {
    if event.is_null() {
        return;
    }
    let event = unsafe { &*event };
    if event.UserContext.is_null() {
        return;
    }
    // Arc<ConsumerContext> is retained until ProcessTrace has returned/joined.
    let context = unsafe { &*event.UserContext.cast::<ConsumerContext>() };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| context.event(event))).is_err() {
        context.fail("Detailed CPU trace callback failed");
    }
}

unsafe extern "system" fn buffer_callback(logfile: *mut EVENT_TRACE_LOGFILEW) -> u32 {
    if logfile.is_null() {
        return 0;
    }
    let logfile = unsafe { &*logfile };
    if logfile.Context.is_null() {
        return 0;
    }
    let context = unsafe { &*logfile.Context.cast::<ConsumerContext>() };
    u32::from(!context.stopping.load(Ordering::Acquire))
}

/// Scheduler residency of the selected exact process instances on each CPU.
/// Starts its own bounded real-time ETW system logger; it never controls a
/// foreign logger and never writes ETL files, captures stacks or loads symbols.
/// Construct, sample, stop and drop on the application's telemetry worker.
/// Samples end at delivered ETW time, so delivery latency can produce None.
pub struct DetailedTrace {
    session: OwnedSession,
    consumer: Option<PROCESSTRACE_HANDLE>,
    context: Arc<ConsumerContext>,
    worker: Option<JoinHandle<()>>,
    targets: BTreeMap<u32, (u64, Option<PinnedProcess>)>,
    samples: Vec<ProcessCpuSample>,
    valid: Vec<Option<bool>>,
    contributions: Vec<super::CpuContribution>,
    history_frames: Vec<super::CpuHistoryFrame>,
    history_targets: Vec<super::timeline::Target>,
    history_states: Vec<(u32, u64, Option<bool>)>,
    next_cut: u64,
    frequency: i64,
}

#[derive(Clone, Copy)]
pub struct CpuHistoryTick {
    pub id: u64,
    pub seconds: f64,
    pub timestamp: std::time::SystemTime,
}

/// Exact-instance scheduler time, reused in place on each 1 Hz sample.
pub struct ProcessCpuSample {
    pub pid: u32,
    pub token: u64,
    pub cpus: Vec<Option<f32>>,
}

impl DetailedTrace {
    pub fn start_history() -> Result<Self, String> {
        let mut trace = Self::start_system(&[])?;
        trace
            .context
            .accounting
            .lock()
            .map_err(|_| "CPU accounting lock failed")?
            .enable_timeline()?;
        trace
            .history_frames
            .try_reserve_exact(super::timeline::PENDING_INTERVALS)
            .map_err(|_| "Not enough CPU frame memory")?;
        trace
            .history_targets
            .try_reserve_exact(1024)
            .map_err(|_| "Not enough CPU target memory")?;
        trace
            .history_states
            .try_reserve_exact(1024)
            .map_err(|_| "Not enough CPU target memory")?;
        unsafe { QueryPerformanceFrequency(&mut trace.frequency) }.map_err(|e| e.to_string())?;
        if trace.frequency <= 0 {
            return Err("CPU event clock is unavailable".into());
        }
        Ok(trace)
    }
    /// Register the original wall/QPC boundary before sampling other counters.
    /// Target mutations take effect only after the old interval closes.
    pub fn history_tick(&mut self, targets: &[(u32, u64)]) -> Result<CpuHistoryTick, String> {
        if targets.len() > 1024
            || targets.iter().any(|(pid, token)| *pid == 0 || *token == 0)
            || targets.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err("Invalid exact CPU history target set".into());
        }
        let mut observed_at = 0;
        unsafe { QueryPerformanceCounter(&mut observed_at) }.map_err(|e| e.to_string())?;
        self.history_states.clear();
        self.history_states
            .extend(self.targets.iter().map(|(&pid, (token, pin))| {
                (pid, *token, pin.as_ref().and_then(|pin| pin.alive().ok()))
            }));
        self.targets
            .retain(|pid, (token, _)| targets.binary_search(&(*pid, *token)).is_ok());
        for &(pid, token) in targets {
            let entry = self.targets.entry(pid).or_insert((token, None));
            if entry.1.is_none() {
                entry.1 = PinnedProcess::open(pid, token).ok();
            }
        }
        self.history_targets.clear();
        self.history_targets
            .extend(self.targets.iter().map(|(&pid, (token, pin))| {
                let alive = self
                    .history_states
                    .iter()
                    .find(|(old, old_token, _)| *old == pid && old_token == token)
                    .and_then(|(_, _, alive)| *alive)
                    .or_else(|| pin.as_ref().and_then(|pin| pin.alive().ok()));
                super::timeline::Target {
                    pid,
                    token: *token,
                    verified_at: pin.as_ref().map_or(0, |pin| {
                        if alive == Some(true) {
                            pin.verified_at.max(observed_at)
                        } else {
                            pin.verified_at
                        }
                    }),
                    alive,
                }
            }));
        let mut accounting = self
            .context
            .accounting
            .lock()
            .map_err(|_| "CPU accounting lock failed")?;
        let mut at = 0;
        unsafe { QueryPerformanceCounter(&mut at) }.map_err(|e| e.to_string())?;
        let timestamp = std::time::SystemTime::now();
        let id = self.next_cut;
        self.next_cut = self
            .next_cut
            .checked_add(1)
            .ok_or("CPU interval identity exhausted")?;
        accounting.queue_cut(id, at, &self.history_targets, &self.history_states)?;
        Ok(CpuHistoryTick {
            id,
            seconds: at as f64 / self.frequency as f64,
            timestamp,
        })
    }
    /// Exchange preallocated frame buffers. Call once per telemetry tick.
    pub fn history_frames(&mut self) -> Result<&[super::CpuHistoryFrame], String> {
        self.check_health()?;
        self.context
            .accounting
            .lock()
            .map_err(|_| "CPU accounting lock failed")?
            .exchange_timeline_frames(&mut self.history_frames)?;
        Ok(&self.history_frames)
    }
    fn check_health(&self) -> Result<(), String> {
        if self.context.stopping.load(Ordering::Acquire) {
            return Err("Detailed CPU monitoring is stopped".into());
        }
        let properties = self.session.query()?;
        if properties.value.EventsLost != 0
            || properties.value.LogBuffersLost != 0
            || properties.value.RealTimeBuffersLost != 0
        {
            self.context
                .fail("ETW lost scheduler events or buffers; CPU history has a gap");
        }
        if super::logical_cpu_count() != self.context.cpus {
            self.context
                .fail("Windows CPU topology changed during detailed monitoring");
        }
        Ok(())
    }
    pub fn start(targets: &[(u32, u64)]) -> Result<Self, String> {
        Self::start_inner(targets, false)
    }
    /// One stream also attributes host CPU work, even with no configured apps.
    pub fn start_system(targets: &[(u32, u64)]) -> Result<Self, String> {
        Self::start_inner(targets, true)
    }
    fn start_inner(targets: &[(u32, u64)], contributors: bool) -> Result<Self, String> {
        if (!contributors && targets.is_empty()) || targets.len() > 1024 {
            return Err(
                "Detailed CPU monitoring requires between 1 and 1024 exact process instances"
                    .into(),
            );
        }
        let mut unique = BTreeMap::new();
        for &(pid, token) in targets {
            if unique
                .insert(pid, token)
                .is_some_and(|previous| previous != token)
            {
                return Err("Conflicting selected process identities".into());
            }
        }
        // Pin identities before starting native tracing. Callbacks never open
        // handles or query process lifetime; admission is checked at sampling.
        let mut first_error = None;
        let pinned: BTreeMap<_, _> = unique
            .into_iter()
            .map(|(pid, token)| {
                let process = match PinnedProcess::open(pid, token) {
                    Ok(process) => Some(process),
                    Err(error) => {
                        first_error.get_or_insert(error);
                        None
                    }
                };
                (pid, (token, process))
            })
            .collect();
        if !contributors && pinned.values().all(|(_, pin)| pin.is_none()) {
            return Err(first_error
                .unwrap_or_else(|| "No verified CPU process instance is available".into()));
        }
        let cpus = coordinates()?.len();
        if cpus > 1024 {
            return Err("Detailed CPU monitoring supports at most 1024 logical CPUs".into());
        }
        let session = OwnedSession::start(cpus, contributors)?;
        let mut accounting = SharedAccounting::new(cpus);
        if contributors {
            accounting.enable_contributors();
        }
        let context = Arc::new(ConsumerContext {
            accounting: Mutex::new(accounting),
            stopping: AtomicBool::new(false),
            cpus,
        });
        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: PWSTR(session.name.as_ptr().cast_mut()),
            Context: Arc::as_ptr(&context).cast_mut().cast(),
            BufferCallback: Some(buffer_callback),
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
            | PROCESS_TRACE_MODE_EVENT_RECORD
            | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
        logfile.Anonymous2.EventRecordCallback = Some(event_callback);
        let consumer = unsafe { OpenTraceW(&mut logfile) };
        if consumer.Value == u64::MAX {
            return Err(format!(
                "Open own CPU trace: {}",
                windows::core::Error::from_thread()
            ));
        }
        let mut trace = Self {
            session,
            consumer: Some(consumer),
            context,
            worker: None,
            samples: pinned
                .iter()
                .map(|(&pid, &(token, _))| ProcessCpuSample {
                    pid,
                    token,
                    cpus: vec![None; cpus],
                })
                .collect(),
            targets: pinned,
            valid: Vec::new(),
            contributions: Vec::new(),
            history_frames: Vec::new(),
            history_targets: Vec::new(),
            history_states: Vec::new(),
            next_cut: 0,
            frequency: 0,
        };
        let consumer_context = Arc::clone(&trace.context);
        trace.worker = Some(
            std::thread::Builder::new()
                .name("cpu-load-etw".into())
                .spawn(move || {
                    let result = unsafe { ProcessTrace(&[consumer], None, None) };
                    if !consumer_context.stopping.load(Ordering::Acquire) {
                        consumer_context.fail(format!(
                            "Detailed CPU trace ended unexpectedly (Windows status 0x{:08X})",
                            result.0
                        ));
                    }
                })
                .map_err(|error| format!("Start CPU trace consumer: {error}"))?,
        );
        let mut accounting = trace
            .context
            .accounting
            .lock()
            .map_err(|_| "CPU accounting lock failed")?;
        accounting.set_processes(
            &trace
                .targets
                .iter()
                .filter_map(|(&pid, (_, pin))| pin.as_ref().map(|_| pid))
                .collect::<Vec<_>>(),
        );
        for (&pid, (_, pin)) in &trace.targets {
            if let Some(pin) = pin {
                accounting.confirm_alive(pid, pin.verified_at);
            }
        }
        drop(accounting);
        Ok(trace)
    }

    /// Changes only admitted process slots. Thread rundown is retained by the
    /// common trace, so existing threads can be attributed without a restart.
    pub fn update_targets(&mut self, targets: &[(u32, u64)]) -> Result<(), String> {
        if self.frequency != 0 {
            return Err("History targets must be changed at an original interval boundary".into());
        }
        if targets.len() > 1024 {
            return Err("Too many tracked CPU process instances".into());
        }
        let mut desired = BTreeMap::new();
        for &(pid, token) in targets {
            if pid == 0 || token == 0 || desired.insert(pid, token).is_some_and(|old| old != token)
            {
                return Err("Conflicting CPU process identities".into());
            }
        }
        {
            let mut accounting = self
                .context
                .accounting
                .lock()
                .map_err(|_| "CPU accounting lock failed")?;
            self.targets.retain(|pid, (token, _)| {
                if desired.get(pid) == Some(token) {
                    true
                } else {
                    accounting.remove_process(*pid);
                    false
                }
            });
        }
        for (&pid, &token) in &desired {
            let entry = self.targets.entry(pid).or_insert((token, None));
            if entry.1.is_none() {
                entry.1 = PinnedProcess::open(pid, token).ok();
            }
        }
        // Reuse sample vectors for unchanged instances; allocation belongs to
        // membership changes, never the scheduler callback or recorded point.
        self.samples
            .retain(|sample| desired.get(&sample.pid) == Some(&sample.token));
        for (&pid, &token) in &desired {
            if !self
                .samples
                .iter()
                .any(|sample| sample.pid == pid && sample.token == token)
            {
                self.samples.push(ProcessCpuSample {
                    pid,
                    token,
                    cpus: vec![None; self.context.cpus],
                });
            }
        }
        self.samples
            .sort_unstable_by_key(|sample| (sample.pid, sample.token));
        self.valid.resize(self.samples.len(), None);
        let admitted: Vec<_> = self
            .targets
            .iter()
            .filter_map(|(&pid, (_, pin))| pin.as_ref().map(|_| pid))
            .collect();
        let mut accounting = self
            .context
            .accounting
            .lock()
            .map_err(|_| "CPU accounting lock failed")?;
        accounting.set_processes(&admitted);
        for (&pid, (_, pin)) in &self.targets {
            if let Some(pin) = pin {
                accounting.confirm_alive(pid, pin.verified_at);
            }
        }
        Ok(())
    }

    pub fn sample_processes(&mut self) -> Result<&[ProcessCpuSample], String> {
        if self.frequency != 0 {
            return Err("History samples must be read as completed original intervals".into());
        }
        if self.context.stopping.load(Ordering::Acquire) {
            return Err("Detailed CPU monitoring is stopped".into());
        }
        let properties = self.session.query()?;
        if properties.value.EventsLost != 0
            || properties.value.LogBuffersLost != 0
            || properties.value.RealTimeBuffersLost != 0
        {
            self.context
                .fail("ETW lost scheduler events or buffers; detailed CPU data is unavailable");
        }
        if super::logical_cpu_count() != self.context.cpus {
            self.context
                .fail("Windows CPU topology changed during detailed monitoring");
        }
        self.valid.clear();
        let mut observed_at = 0;
        unsafe { QueryPerformanceCounter(&mut observed_at) }.map_err(|e| e.to_string())?;
        self.valid.extend(self.samples.iter().map(|sample| {
            self.targets
                .get(&sample.pid)
                .and_then(|(_, pin)| pin.as_ref())
                .and_then(|p| p.alive().ok())
        }));
        let mut accounting = self
            .context
            .accounting
            .lock()
            .map_err(|_| "Detailed CPU accounting lock failed".to_string())?;
        for (sample, &alive) in self.samples.iter_mut().zip(&self.valid) {
            if alive == Some(true) {
                accounting.confirm_alive(sample.pid, observed_at);
            }
            if alive.is_some() {
                accounting.sample_into(sample.pid, &mut sample.cpus)?;
                if alive == Some(false) && !accounting.confirmed_exit(sample.pid) {
                    sample.cpus.fill(None);
                }
            } else {
                sample.cpus.fill(None);
            }
        }
        drop(accounting);
        // Native lifetime checks never hold the scheduler callback's lock.
        self.valid.clear();
        self.valid.extend(self.samples.iter().map(|sample| {
            self.targets[&sample.pid]
                .1
                .as_ref()
                .and_then(|p| p.alive().ok())
        }));
        let accounting = self
            .context
            .accounting
            .lock()
            .map_err(|_| "CPU accounting lock failed")?;
        for (sample, &alive) in self.samples.iter_mut().zip(&self.valid) {
            if alive.is_none() || (alive == Some(false) && !accounting.confirmed_exit(sample.pid)) {
                sample.cpus.fill(None);
            }
        }
        Ok(&self.samples)
    }

    pub fn sample(&mut self) -> Result<Vec<Option<f32>>, String> {
        let cpus = self.context.cpus;
        let samples = self.sample_processes()?;
        Ok((0..cpus)
            .map(|cpu| {
                samples
                    .iter()
                    .try_fold(0.0f32, |sum, sample| sample.cpus[cpu].map(|v| sum + v))
                    .map(|v| v.min(100.0))
            })
            .collect())
    }
    /// Call after sample_processes has validated loss/topology for this tick.
    pub fn sample_contributors(&mut self) -> Result<&[super::CpuContribution], String> {
        if self.frequency != 0 {
            return Err("History contributors must be read with their original interval".into());
        }
        self.context
            .accounting
            .lock()
            .map_err(|_| "CPU contributor lock failed")?
            .sample_contributors(&mut self.contributions)?;
        Ok(&self.contributions)
    }
    pub fn process_identity(&self, pid: u32, token: u64) -> Option<Arc<super::CpuProcess>> {
        use windows::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
        let (actual, pin) = self.targets.get(&pid)?;
        let pin = pin.as_ref().filter(|_| *actual == token)?;
        if !pin.alive().ok()? {
            return None;
        }
        let mut path = [0u16; 32768];
        let mut size = path.len() as u32;
        let name = if unsafe {
            QueryFullProcessImageNameW(
                pin.native(),
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut size,
            )
        }
        .is_ok()
        {
            String::from_utf16_lossy(&path[..size as usize])
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or("Name unavailable")
                .chars()
                .filter(|c| !c.is_control())
                .take(255)
                .collect::<String>()
        } else {
            "Name unavailable".into()
        };
        if !pin.alive().ok()? {
            return None;
        }
        Some(Arc::new(super::CpuProcess {
            pid,
            instance: token,
            name: name.into(),
        }))
    }

    pub fn memory_usage(&self) -> (usize, usize) {
        self.context
            .accounting
            .lock()
            .map(|a| {
                (
                    a.bookkeeping_bytes()
                        + self.contributions.capacity() * size_of::<super::CpuContribution>()
                        + self
                            .contributions
                            .iter()
                            .map(|c| c.cpus.capacity() * size_of::<Option<f32>>())
                            .sum::<usize>(),
                    a.thread_count(),
                )
            })
            .unwrap_or_default()
    }
    pub fn contributors_partial(&self) -> bool {
        self.context
            .accounting
            .lock()
            .map_or(true, |a| a.contributors_partial())
    }

    pub fn has_unavailable_targets(&self) -> bool {
        self.targets.values().any(|(_, pin)| pin.is_none())
    }

    /// Stops only this owned session and joins the consumer. Call off the GUI
    /// thread. Explicit stop exposes cleanup failures; Drop is a final fallback.
    pub fn stop(&mut self) -> Result<(), String> {
        self.context.stopping.store(true, Ordering::Release);
        let close = self
            .consumer
            .take()
            .map(|consumer| unsafe { CloseTrace(consumer) });
        let stop = self.session.stop();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "CPU trace consumer thread failed".to_string())?;
        }
        stop?;
        if let Some(result) = close
            && result != ERROR_SUCCESS
            && result != ERROR_CTX_CLOSE_PENDING
        {
            return status("Close own CPU trace consumer", result.0);
        }
        Ok(())
    }
}

impl Drop for DetailedTrace {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};

    use super::*;
    use crate::windows::cpu_load::{CpuSampler, ProcessSampler};

    #[test]
    #[ignore = "opt-in elevated original-time interval/late-drain probe; about nine seconds"]
    fn native_original_interval_history_survives_late_drain() {
        let pid = unsafe { GetCurrentProcessId() };
        let token = crate::OS::get_process_instance_token(pid).unwrap();
        let mut trace = DetailedTrace::start_history().unwrap();
        let load = std::thread::spawn(|| {
            let until = Instant::now() + Duration::from_secs(8);
            let mut value = 1u64;
            while Instant::now() < until {
                for _ in 0..2000 {
                    value =
                        std::hint::black_box(value.wrapping_mul(1664525).wrapping_add(1013904223));
                }
            }
        });
        let mut ticks = Vec::new();
        for _ in 0..7 {
            ticks.push(trace.history_tick(&[(pid, token)]).unwrap());
            std::thread::sleep(Duration::from_secs(1));
        }
        // Do not consume or recycle any completed frame during those cuts.
        std::thread::sleep(Duration::from_millis(500));
        let frequency = trace.frequency as f64;
        let frames = trace.history_frames().unwrap();
        assert!(
            frames.len() >= 5,
            "late drain must retain original completed cuts"
        );
        let mut positive = 0;
        for frame in frames.iter().filter(|f| f.available) {
            assert!(
                (frame.end as f64 / frequency - ticks[frame.id as usize].seconds).abs() < 0.000001
            );
            for value in &frame.tracked {
                let host: f32 = frame
                    .contributors
                    .iter()
                    .filter(|r| r.cpu == value.cpu && r.process.pid == pid)
                    .map(|r| r.load)
                    .sum();
                assert!(
                    (host - value.load).abs() < 0.01,
                    "same original interval must give matching tracked and host attribution"
                );
                positive += 1;
            }
        }
        assert!(positive > 0, "no controlled measured work");
        eprintln!(
            "ORIGINAL_INTERVALS retained={} positive_cells={positive} buffers={}",
            frames.len(),
            super::super::history_buffer_bytes(super::super::logical_cpu_count()).unwrap()
        );
        trace.history_frames().unwrap();
        assert!(
            trace.history_frames().unwrap().is_empty(),
            "completed cuts publish only once"
        );
        trace.stop().unwrap();
        assert!(trace.history_frames().is_err());
        load.join().unwrap();
    }

    #[test]
    #[ignore = "opt-in elevated read-only per-PID missing-sample/lifecycle diagnosis; fourteen seconds"]
    fn native_diagnose_browser_sample_availability() {
        use crate::OS;
        use std::os::windows::process::CommandExt;
        let mut targets = Vec::new();
        for name in ["chrome", "ChatGPT"] {
            for pid in OS::find_pids_by_name(name) {
                match OS::get_process_instance_token(pid) {
                    Ok(token) => targets.push((pid, token)),
                    Err(error) => eprintln!("ADMISSION pid={pid} reason={error}"),
                }
            }
        }
        let pid = unsafe { GetCurrentProcessId() };
        targets.push((pid, OS::get_process_instance_token(pid).unwrap()));
        targets.sort_unstable();
        targets.dedup();
        let mut trace = DetailedTrace::start_system(&targets).unwrap();
        eprintln!("DIAG targets={} cpus={}", targets.len(), trace.context.cpus);
        for (pid, (token, pin)) in &trace.targets {
            if pin.is_none() {
                eprintln!(
                    "PIN pid={pid} reason={:?}",
                    PinnedProcess::open(*pid, *token).err()
                );
            }
        }
        let mut child = None;
        let mut child_pid = None;
        let mut saw_child_work = false;
        let mut saw_child_zero_after_exit = false;
        for tick in 1..=14 {
            if tick == 4 {
                let process = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--ignored",
                        "--exact",
                        "windows::cpu_load::trace::tests::native_exit_diagnosis_child",
                        "--nocapture",
                    ])
                    .creation_flags(0x08000000)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                let pid = process.id();
                targets.push((pid, OS::get_process_instance_token(pid).unwrap()));
                trace.update_targets(&targets).unwrap();
                child_pid = Some(pid);
                child = Some(process);
            }
            std::thread::sleep(Duration::from_secs(1));
            let samples = trace.sample_processes().unwrap();
            if let Some(sample) = samples.iter().find(|s| Some(s.pid) == child_pid) {
                saw_child_work |= sample.cpus.iter().flatten().any(|v| *v > 0.0);
                let exited = child.as_mut().unwrap().try_wait().unwrap().is_some();
                if exited && sample.cpus.iter().all(|v| *v == Some(0.0)) {
                    saw_child_zero_after_exit = true;
                }
                eprintln!("CHILD tick={tick} exited={exited} cpus={:?}", sample.cpus);
            }
            let complete = samples
                .iter()
                .filter(|s| s.cpus.iter().all(Option::is_some))
                .count();
            let unknown: Vec<_> = samples
                .iter()
                .filter(|s| s.cpus.iter().any(Option::is_none))
                .map(|s| (s.pid, s.cpus.iter().filter(|v| v.is_none()).count()))
                .collect();
            eprintln!("TICK {tick} complete={complete} unknown={unknown:?}");
            for &(pid, _) in &unknown {
                let reason = match &trace.targets[&pid].1 {
                    None => "pin unavailable".into(),
                    Some(pin) => format!("alive={:?}", pin.alive()),
                };
                eprintln!("MISSING pid={pid} {reason}");
            }
            eprintln!(
                "ACCOUNT {}",
                trace.context.accounting.lock().unwrap().sample_diagnostic()
            );
        }
        trace.stop().unwrap();
        if let Some(mut child) = child {
            assert!(child.wait().unwrap().success());
        }
        assert!(saw_child_work, "controlled child CPU work was not measured");
        assert!(
            saw_child_zero_after_exit,
            "a confirmed exited instance remained unknown instead of known zero"
        );
    }

    #[test]
    #[ignore = "controlled short-lived load child for the availability diagnosis"]
    fn native_exit_diagnosis_child() {
        let until = Instant::now() + Duration::from_secs(5);
        let mut value = 1u64;
        while Instant::now() < until {
            for _ in 0..10000 {
                value =
                    std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
        }
        std::hint::black_box(value);
    }

    #[test]
    #[ignore = "opt-in real Windows ETW probe; run elevated, consumes CPU for five seconds"]
    fn native_cpu_load_samples_exact_process_and_stops_only_owned_trace() {
        let pid = unsafe { GetCurrentProcessId() };
        let mut created = windows::Win32::Foundation::FILETIME::default();
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        unsafe {
            super::super::GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        }
        .unwrap();
        let token = u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime);
        assert!(ProcessSampler::open(pid, token.wrapping_add(1)).is_err());
        let mut process = ProcessSampler::open(pid, token).unwrap();
        let mut cpu = CpuSampler::new().unwrap();
        assert_eq!(process.sample().unwrap(), None);
        assert!(cpu.sample().unwrap().iter().all(Option::is_none));
        let mut trace = DetailedTrace::start(&[(pid, token)]).unwrap();
        // A second independently owned session proves stopping the first does
        // not stop another logger. No pre-existing logger is controlled.
        let mut sentinel = OwnedSession::start(trace.context.cpus, false).unwrap();
        let busy = std::thread::spawn(|| {
            let until = Instant::now() + Duration::from_secs(5);
            let mut value = 1u64;
            while Instant::now() < until {
                for _ in 0..10_000 {
                    value = std::hint::black_box(
                        value.wrapping_mul(6364136223846793005).wrapping_add(1),
                    );
                }
            }
            std::hint::black_box(value)
        });
        let mut saw_detail = false;
        let mut saw_process = false;
        for _ in 0..4 {
            std::thread::sleep(Duration::from_secs(1));
            let total = cpu.sample().unwrap();
            assert!(
                !total.is_empty()
                    && total
                        .iter()
                        .flatten()
                        .all(|value| (0.0..=100.0).contains(value))
            );
            let aggregate = process.sample().unwrap();
            let detail = trace.sample().unwrap();
            println!(
                "CPU load native: aggregate={aggregate:?}, total={total:?}, process per CPU={detail:?}"
            );
            saw_process |= aggregate.is_some_and(|value| value > 0.0 && value <= 100.0);
            saw_detail |= detail
                .iter()
                .flatten()
                .any(|value| *value > 0.0 && *value <= 100.0);
        }
        // One inaccessible/unverified instance cannot stop healthy siblings.
        trace
            .update_targets(&[(pid, token), (u32::MAX, 1)])
            .unwrap();
        let samples = trace.sample_processes().unwrap();
        assert!(
            samples
                .iter()
                .find(|s| s.pid == u32::MAX)
                .unwrap()
                .cpus
                .iter()
                .all(Option::is_none)
        );
        trace.update_targets(&[(pid, token)]).unwrap();
        let name = trace.session.name.clone();
        let guid = trace.session.guid;
        let started = Instant::now();
        trace.stop().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "CPU trace shutdown was not prompt"
        );
        assert!(trace.worker.is_none() && trace.consumer.is_none());
        let mut properties = Properties::new(&name, guid);
        let absent = unsafe {
            ControlTraceW(
                CONTROLTRACE_HANDLE::default(),
                PCWSTR(name.as_ptr()),
                &mut properties.value,
                EVENT_TRACE_CONTROL_QUERY,
            )
        };
        assert_eq!(
            absent,
            windows::Win32::Foundation::ERROR_WMI_INSTANCE_NOT_FOUND,
            "Own CPU logger survived shutdown"
        );
        sentinel.query().unwrap();
        sentinel.stop().unwrap();
        busy.join().unwrap();
        assert!(
            saw_process,
            "No aggregate process CPU activity was observed"
        );
        assert!(
            saw_detail,
            "No actual per-CPU scheduler residency was observed"
        );
    }
}
