//! Explicit bounded CPU-history exports. No sampling, periodic work or remote file paths.
use super::history::{HistoryStore, SampleTime};
use crate::app::shared::ids::RuleId;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime},
};

const MAX_BYTES: u64 = 512 * 1024 * 1024;
const LIFETIME: Duration = Duration::from_secs(600);
const HEADER: &str = "time_utc,elapsed_seconds,record_type,scope,process,pid,historical_lifetime_id,cpu,cpu_percent,quality,delivery,gap_reason,parked,rule_id,run_id,native_interval_id,begin_qpc,end_qpc,qpc_frequency,process_attribution\r\n";
pub(super) struct ExportRange {
    pub epoch: u64,
    pub times: Vec<SampleTime>,
}
pub(super) type ExportRevision = (u64, Option<super::contributors::ExportFrameToken>);
pub(super) struct ExportLine {
    pub time: SampleTime,
    pub interval: Option<(u64, i64, i64)>,
    pub delivery: &'static str,
    pub gap: Option<&'static str>,
    pub scope: &'static str,
    pub process: Option<Arc<os_api::cpu_load::CpuProcess>>,
    pub rule: Option<Arc<RuleId>>,
    pub run: Option<Arc<str>>,
    pub cpu: usize,
    pub value: Option<f32>,
    pub quality: &'static str,
    pub attribution: &'static str,
    pub parked: Option<bool>,
}
pub(super) fn timestamp(time: SystemTime) -> String {
    crate::app::features::diagnostics::crash_reports::format_utc_timestamp(time)
}
fn iso_timestamp(time: SystemTime) -> String {
    let t = timestamp(time);
    format!(
        "{}-{}-{}T{}:{}:{}",
        &t[0..4],
        &t[4..6],
        &t[6..8],
        &t[9..11],
        &t[11..13],
        &t[13..]
    )
}
pub(super) fn filename(prefix: &str, extension: &str) -> String {
    format!("{prefix}-{}.{}", timestamp(SystemTime::now()), extension)
}

fn cell(value: &str) -> String {
    // Excel must not execute a process basename as a spreadsheet formula.
    let safe = if value.starts_with(['=', '+', '-', '@', '\t', '\r', '\n']) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    format!("\"{}\"", safe.replace('"', "\"\""))
}
fn write_line(
    out: &mut impl Write,
    row: &ExportLine,
    frequency: Option<i64>,
) -> std::io::Result<()> {
    let process = row.process.as_ref();
    let number = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_default();
    let interval = row.interval;
    let values = [
        cell(&iso_timestamp(row.time.wall)),
        row.time.seconds.to_string(),
        cell(if process.is_some() { "process" } else { "cpu" }),
        cell(row.scope),
        cell(process.map(|p| p.name.as_ref()).unwrap_or("")),
        number(process.map(|p| p.pid as u64)),
        number(process.map(|p| p.instance)),
        row.cpu.to_string(),
        row.value
            .filter(|v| v.is_finite())
            .map(|v| v.to_string())
            .unwrap_or_default(),
        cell(row.quality),
        cell(row.delivery),
        cell(row.gap.unwrap_or("")),
        row.parked.map(|v| v.to_string()).unwrap_or_default(),
        cell(row.rule.as_ref().map(|r| r.0.as_str()).unwrap_or("")),
        cell(row.run.as_deref().unwrap_or("")),
        number(interval.map(|v| v.0)),
        interval.map(|v| v.1.to_string()).unwrap_or_default(),
        interval.map(|v| v.2.to_string()).unwrap_or_default(),
        frequency.map(|v| v.to_string()).unwrap_or_default(),
        cell(row.attribution),
    ];
    out.write_all(values.join(",").as_bytes())?;
    out.write_all(b"\r\n")
}
pub(super) struct Artifact {
    pub file: Mutex<File>,
    pub name: String,
    pub bytes: u64,
    pub rows: u64,
}
fn read_history<T>(
    history: &RwLock<HistoryStore>,
    cancelled: &AtomicBool,
    mut read: impl FnMut(&HistoryStore) -> Result<T, String>,
) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Export cancelled because the app is closing.".into());
        }
        match history.try_read() {
            Ok(history) => return read(&history),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("CPU history is unavailable.".into())
            }
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(_) => return Err("CPU history is busy. Try again.".into()),
        }
    }
}
pub(super) fn capture(
    history: &RwLock<HistoryStore>,
    cancelled: &AtomicBool,
) -> Result<Artifact, String> {
    let range = read_history(history, cancelled, |history| history.export_range())?;
    let file = os_api::agent::create_export_file()?;
    let mut out = std::io::BufWriter::new(file);
    out.write_all(b"\xef\xbb\xbf")
        .and_then(|_| out.write_all(HEADER.as_bytes()))
        .map_err(|e| e.to_string())?;
    let frequency = os_api::cpu_load::history_frequency().ok();
    let mut rows = 0;
    let mut bytes = HEADER.len() as u64 + 3;
    for time in &range.times {
        let mut offset = 0;
        let mut revision = None;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err("Export cancelled because the app is closing.".into());
            }
            let (lines, rev, more) = read_history(history, cancelled, |history| {
                history.export_page(&range, *time, offset, revision)
            })?;
            revision = Some(rev);
            offset += lines.len();
            for line in lines {
                let mut encoded = Vec::with_capacity(256);
                write_line(&mut encoded, &line, frequency).map_err(|e| e.to_string())?;
                bytes += encoded.len() as u64;
                if bytes > MAX_BYTES {
                    return Err("CSV export exceeds 512 MiB. Choose a shorter history.".into());
                }
                out.write_all(&encoded).map_err(|e| e.to_string())?;
                rows += 1;
            }
            if !more {
                break;
            }
            std::thread::yield_now();
        }
    }
    let file = out.into_inner().map_err(|e| e.to_string())?;
    Ok(Artifact {
        file: Mutex::new(file),
        name: filename("cpu-history", "csv"),
        bytes,
        rows,
    })
}
enum State {
    Preparing,
    Ready(Arc<Artifact>),
    Failed(String),
}
struct Job {
    id: String,
    created: Instant,
    state: State,
}
#[derive(Default)]
pub(super) struct ExportManager {
    job: Mutex<Option<Job>>,
    capturing: AtomicBool,
    pub cancelled: AtomicBool,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    export_id: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
}
impl ExportManager {
    pub fn capture(&self, history: &RwLock<HistoryStore>) -> Result<Artifact, String> {
        if self
            .capturing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Another CPU history export is in progress.".into());
        }
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _done = Done(&self.capturing);
        capture(history, &self.cancelled)
    }
    pub fn start(
        self: &Arc<Self>,
        history: Arc<RwLock<HistoryStore>>,
        ctx: eframe::egui::Context,
    ) -> Result<String, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut state = self.job.try_lock().map_err(|_| "Export is busy")?;
        if state
            .as_ref()
            .is_some_and(|j| matches!(j.state, State::Preparing))
        {
            return Err("An export is already being prepared.".into());
        }
        let id = format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        *state = Some(Job {
            id: id.clone(),
            created: Instant::now(),
            state: State::Preparing,
        });
        let manager = self.clone();
        let worker_id = id.clone();
        let result = std::thread::Builder::new()
            .name("cpu-history-export".into())
            .spawn(move || {
                let result = manager.capture(&history);
                if let Ok(mut job) = manager.job.lock() {
                    if let Some(job) = job.as_mut().filter(|j| j.id == worker_id) {
                        job.state = match result {
                            Ok(a) => State::Ready(Arc::new(a)),
                            Err(e) => State::Failed(e),
                        };
                    }
                }
                ctx.request_repaint();
            });
        if let Err(error) = result {
            if let Some(job) = state.as_mut() {
                job.state = State::Failed(error.to_string());
            }
            return Err(error.to_string());
        }
        Ok(id)
    }
    pub fn artifact(&self, id: &str) -> Result<Option<Arc<Artifact>>, String> {
        let mut state = self.job.try_lock().map_err(|_| "Export is busy")?;
        if state.as_ref().is_some_and(|j| {
            j.created.elapsed() >= LIFETIME && !matches!(j.state, State::Preparing)
        }) {
            *state = None;
        }
        let job = state
            .as_ref()
            .filter(|j| j.id == id)
            .ok_or("Export expired or was replaced. Start a new export.")?;
        match &job.state {
            State::Preparing => Ok(None),
            State::Ready(a) => Ok(Some(a.clone())),
            State::Failed(e) => Err(e.clone()),
        }
    }
    pub fn query(
        self: &Arc<Self>,
        history: Arc<RwLock<HistoryStore>>,
        ctx: eframe::egui::Context,
        args: Value,
    ) -> Result<Value, String> {
        let q: Query = serde_json::from_value(args).map_err(|e| e.to_string())?;
        let limit = q.limit.unwrap_or(32768);
        if !(4..=32768).contains(&limit) {
            return Err("Export chunk limit must be 4..32768 bytes.".into());
        }
        let after = q
            .after
            .as_deref()
            .unwrap_or("0")
            .parse::<u64>()
            .map_err(|_| "Invalid byte cursor")?;
        let id = match q.export_id {
            Some(id) if id.len() <= 100 => id,
            Some(_) => return Err("Invalid export ID".into()),
            None => {
                if after != 0 {
                    return Err("An export_id is required with a byte cursor.".into());
                }
                self.start(history, ctx)?
            }
        };
        let artifact = self.artifact(&id)?;
        let mut reply = json!({"export_id":id,"status":"preparing","filename":null,"mime_type":"text/csv","encoding":"UTF-8 with BOM","schema_version":1,"bytes":null,"rows":null,"after":after.to_string(),"next_after":after.to_string(),"eof":false,"text":"","expires_after_seconds":600,"instructions":"Poll with export_id until ready, then append text exactly as UTF-8 bytes using next_after until eof. No newlines or extra BOMs. CSV uses commas/CRLF; blank cpu_percent is unavailable, never zero. cpu_percent is percent of one logical CPU. Scopes are separate views: do not sum system/rule/run copies. Process rows retain positive observed work, not proof of activity or existence in every interval. Historical lifetime IDs cannot control live processes. Spreadsheet-formula basenames have an apostrophe prefix."});
        if let Some(a) = artifact {
            if after > a.bytes {
                return Err("Export byte cursor exceeds file size.".into());
            }
            let mut data = vec![0; limit.min((a.bytes - after) as usize)];
            {
                let mut file = a.file.lock().map_err(|_| "Export read failed")?;
                file.seek(SeekFrom::Start(after))
                    .and_then(|_| file.read_exact(&mut data))
                    .map_err(|e| e.to_string())?;
            }
            let valid = match std::str::from_utf8(&data) {
                Ok(_) => data.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => {
                    return Err("Invalid UTF-8 cursor. Use the exact returned next_after.".into())
                }
            };
            data.truncate(valid);
            let next = after + valid as u64;
            reply["status"] = json!("ready");
            reply["filename"] = json!(a.name);
            reply["bytes"] = json!(a.bytes.to_string());
            reply["rows"] = json!(a.rows.to_string());
            reply["next_after"] = json!(next.to_string());
            reply["eof"] = json!(next == a.bytes);
            reply["text"] = json!(String::from_utf8(data).map_err(|e| e.to_string())?);
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        contributors::{Contribution, ContributorHistory},
        history::GapReason,
    };
    use super::*;
    use std::{collections::HashMap, time::UNIX_EPOCH};

    fn fixture() -> RwLock<HistoryStore> {
        let mut store = HistoryStore::default();
        store.configure(2, 30, &[]).unwrap();
        for (second, instance) in [(1, 111), (2, 222)] {
            let time = SampleTime {
                seconds: second as f64,
                wall: UNIX_EPOCH + Duration::from_secs(second),
            };
            store.push(
                time.seconds,
                time.wall,
                &[Some(25.0), Some(0.0)],
                &[Some(false), Some(true)],
                &HashMap::new(),
            );
            let process = Arc::new(os_api::cpu_load::CpuProcess {
                pid: 7,
                instance,
                name: "тест,\"app\".exe".into(),
            });
            let mut rows = vec![Contribution {
                process: process.clone(),
                rule: None,
                run: None,
                cpu: 0,
                load: 25.0,
            }];
            if second == 1 {
                rows.push(Contribution {
                    process,
                    rule: Some(Arc::new(RuleId("r1".into()))),
                    run: None,
                    cpu: 0,
                    load: 25.0,
                });
            }
            store.contributors.push(time, &rows, true);
        }
        let lost = SampleTime {
            seconds: 3.0,
            wall: UNIX_EPOCH + Duration::from_secs(3),
        };
        store.push_pending(lost, &[]);
        store.mark_lost_reason(lost, GapReason::ContributorCapacity);
        RwLock::new(store)
    }
    fn bytes(a: &Artifact) -> Vec<u8> {
        let mut file = a.file.lock().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn csv_preserves_original_times_pid_reuse_scopes_and_missing_values() {
        let artifact = capture(&fixture(), &AtomicBool::new(false)).unwrap();
        let csv = String::from_utf8(bytes(&artifact)).unwrap();
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("1970-01-01T00:00:01.000Z"));
        assert!(csv.contains("\"тест,\"\"app\"\".exe\",7,111,0,25,"));
        assert!(csv.contains("\"тест,\"\"app\"\".exe\",7,222,0,25,"));
        assert!(csv.contains("\"process\",\"rule\""));
        assert!(csv
            .lines()
            .filter(|l| l.contains("\"lost\""))
            .all(|l| l.contains(",,\"unavailable\",\"lost\"")));
        assert_eq!(artifact.rows, 15);
        assert_eq!(artifact.bytes, csv.len() as u64);
        assert!(artifact.name.starts_with("cpu-history-"));
        assert!(artifact.name.ends_with(".csv"));
        assert_eq!(cell("=SUM(1,2)"), "\"'=SUM(1,2)\"");
        assert_eq!(cell("a\"b\nc"), "\"a\"\"b\nc\"");
    }
    #[test]
    fn immutable_download_chunks_preserve_utf8_and_outlive_collection_off() {
        let history = Arc::new(fixture());
        let artifact = Arc::new(capture(&history, &AtomicBool::new(false)).unwrap());
        let original = bytes(&artifact);
        let manager = Arc::new(ExportManager::default());
        *manager.job.lock().unwrap() = Some(Job {
            id: "test".into(),
            created: Instant::now(),
            state: State::Ready(artifact),
        });
        *history.write().unwrap() = HistoryStore::default();
        let ctx = eframe::egui::Context::default();
        let mut after = "0".to_string();
        let mut downloaded = Vec::new();
        loop {
            let reply = manager
                .query(
                    history.clone(),
                    ctx.clone(),
                    json!({"export_id":"test","after":after,"limit":13}),
                )
                .unwrap();
            let text = reply["text"].as_str().unwrap();
            assert!(text.len() <= 13);
            downloaded.extend_from_slice(text.as_bytes());
            let next = reply["next_after"].as_str().unwrap().to_string();
            if reply["eof"] == true {
                crate::app::features::mcp::assert_contract("export_cpu_history", &reply);
                break;
            }
            assert!(next.parse::<u64>().unwrap() > after.parse::<u64>().unwrap());
            after = next;
        }
        assert_eq!(downloaded, original);
        assert!(manager
            .query(
                history.clone(),
                ctx.clone(),
                json!({"export_id":"test","limit":32769})
            )
            .is_err());
        assert!(manager
            .query(
                history.clone(),
                ctx.clone(),
                json!({"export_id":"test","after":"999999"})
            )
            .is_err());
        manager.job.lock().unwrap().as_mut().unwrap().created =
            Instant::now() - Duration::from_secs(601);
        assert!(manager
            .query(history, ctx, json!({"export_id":"test"}))
            .is_err());
    }
    #[test]
    fn changed_interval_or_contributor_eviction_cannot_mix_export_pages() {
        let mut history = HistoryStore::default();
        history.configure(80, 30, &[]).unwrap();
        let time = SampleTime {
            seconds: 1.0,
            wall: UNIX_EPOCH,
        };
        history.push_pending(time, &[]);
        let range = history.export_range().unwrap();
        let (rows, revision, more) = history.export_page(&range, time, 0, None).unwrap();
        assert_eq!(rows.len(), 128);
        assert!(more);
        history.mark_lost(time);
        assert!(history
            .export_page(&range, time, 128, Some(revision))
            .is_err());
        let (_, revision, _) = history.export_page(&range, time, 0, None).unwrap();
        history.contributors.push(time, &[], true);
        assert!(history
            .export_page(&range, time, 128, Some(revision))
            .is_err());
        history.contributors = ContributorHistory::new(80, 30).unwrap();
        assert!(capture(&RwLock::new(history), &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn export_preparation_is_explicit_bounded_and_has_a_typed_status() {
        let manager = Arc::new(ExportManager::default());
        let history = Arc::new(fixture());
        let ctx = eframe::egui::Context::default();
        let first = manager
            .query(history.clone(), ctx.clone(), json!({}))
            .unwrap();
        crate::app::features::mcp::assert_contract("export_cpu_history", &first);
        let id = first["export_id"].as_str().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let reply = manager
                .query(history.clone(), ctx.clone(), json!({"export_id":id}))
                .unwrap();
            if reply["status"] == "ready" {
                assert_eq!(reply["eof"], true);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
