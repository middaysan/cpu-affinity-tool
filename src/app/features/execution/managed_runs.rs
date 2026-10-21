//! Exact managed child ownership and optional bounded output; independent of HTTP admission.
use os_api::agent::{LaunchSpec, ManagedProcess};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};
const STREAM_BYTES: usize = 64 * 1024;
const GLOBAL_BYTES: usize = 2 * 1024 * 1024;
// The MCP response includes both structured JSON and its text representation.
// Bound escaped display text as well as characters; operational IDs stay exact.
fn bounded_display(text: &str, max_chars: usize, max_json_bytes: usize) -> String {
    let mut bytes = 0;
    let mut end = 0;
    for (count, (index, ch)) in text.char_indices().enumerate() {
        let cost = match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            ch if ch <= '\u{1f}' => 6,
            ch => ch.len_utf8(),
        };
        if count == max_chars || bytes + cost > max_json_bytes {
            let mut prefix = text[..end].to_owned();
            // Reserve one scalar and three UTF-8 bytes for the visible ellipsis.
            while prefix.chars().count() >= max_chars
                || serde_json::to_string(&prefix).unwrap().len() - 2 + 3 > max_json_bytes
            {
                prefix.pop();
            }
            prefix.push('…');
            return prefix;
        }
        bytes += cost;
        end = index + ch.len_utf8();
    }
    text.to_owned()
}
fn run_label(name: &str) -> String {
    bounded_display(name, 256, 1024)
}

#[derive(serde::Serialize)]
struct RunView<'a> {
    run_id: &'a str,
    name: &'a str,
    pid: u32,
    creation_token: String,
    elevated: bool,
    exit_code: Option<u32>,
    root_exited: Option<bool>,
    exit_error: Option<String>,
    capture: bool,
    capture_complete: bool,
    stdout_eof: Option<bool>,
    stderr_eof: Option<bool>,
}
impl RunView<'_> {
    fn into_json(self) -> Value {
        serde_json::to_value(self).expect("Run view contains only JSON-compatible fields")
    }
}
#[derive(Default)]
struct Stream {
    bytes: VecDeque<u8>,
    base: u64,
    end: u64,
    eof: bool,
    finished: bool,
    error: Option<String>,
}
struct Output {
    streams: [Mutex<Stream>; 2],
    used: Arc<AtomicUsize>,
    stop: AtomicBool,
}
impl Output {
    fn new(used: Arc<AtomicUsize>) -> Self {
        Self {
            streams: Default::default(),
            used,
            stop: AtomicBool::new(false),
        }
    }
    fn append(&self, index: usize, bytes: &[u8]) {
        let mut stream = self.streams[index].lock().unwrap();
        stream.end = stream.end.saturating_add(bytes.len() as u64);
        let bytes = &bytes[bytes.len().saturating_sub(STREAM_BYTES)..];
        let remove = (stream.bytes.len() + bytes.len())
            .saturating_sub(STREAM_BYTES)
            .min(stream.bytes.len());
        stream.bytes.drain(..remove);
        stream.base += remove as u64;
        self.used.fetch_sub(remove, Ordering::Relaxed);
        // Keep the declared Rust 1.95 minimum while newer compilers retire fetch_update.
        let mut previous = self.used.load(Ordering::Relaxed);
        loop {
            let next = (previous + bytes.len()).min(GLOBAL_BYTES);
            match self.used.compare_exchange_weak(
                previous,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => previous = actual,
            }
        }
        let accepted = bytes.len().min(GLOBAL_BYTES.saturating_sub(previous));
        if accepted < bytes.len() {
            let old = stream.bytes.len();
            stream.bytes.clear();
            self.used.fetch_sub(old, Ordering::Relaxed);
        }
        stream.bytes.extend(&bytes[bytes.len() - accepted..]);
        stream.base = stream.end - stream.bytes.len() as u64;
    }
    fn page(&self, index: usize, after: u64, limit: usize) -> Result<Value, String> {
        let stream = self.streams[index].lock().unwrap();
        if after > stream.end {
            return Err("Output cursor is beyond this stream".into());
        }
        let start = after.max(stream.base);
        let bytes: Vec<_> = stream
            .bytes
            .iter()
            .skip((start - stream.base) as usize)
            .take(limit)
            .copied()
            .collect();
        Ok(
            json!({"start":start.to_string(),"next":(start+bytes.len()as u64).to_string(),"end":stream.end.to_string(),"dropped_before":stream.base.to_string(),"cursor_gap":after<stream.base,"eof":stream.eof,"reader_finished":stream.finished,"read_error":stream.error,"text":String::from_utf8_lossy(&bytes),"encoding":"UTF-8 lossy; cursors count original bytes","untrusted_data":true}),
        )
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        for stream in &self.streams {
            if let Ok(stream) = stream.lock() {
                self.used.fetch_sub(stream.bytes.len(), Ordering::Relaxed);
            }
        }
    }
}
struct Run {
    id: String,
    grant: String,
    name: String,
    process: ManagedProcess,
    output: Option<Arc<Output>>,
    readers: Vec<JoinHandle<()>>,
    spec: LaunchSpec,
    saved_rule: bool,
}
impl Run {
    fn view(&self) -> Value {
        let exit = self.process.exit_code();
        let root_exited = exit.as_ref().ok().map(Option::is_some);
        let error = exit
            .as_ref()
            .err()
            .map(|error| bounded_display(error, 128, 256));
        let capture_complete = self.output.as_ref().is_none_or(|o| {
            o.streams.iter().all(|s| {
                let s = s.lock().unwrap();
                s.eof && s.error.is_none()
            })
        });
        RunView {
            run_id: &self.id,
            name: &self.name,
            pid: self.process.pid,
            creation_token: self.process.creation_token.to_string(),
            elevated: self.process.elevated,
            exit_code: exit.ok().flatten(),
            root_exited,
            exit_error: error,
            capture: self.output.is_some(),
            capture_complete,
            stdout_eof: self
                .output
                .as_ref()
                .map(|o| o.streams[0].lock().unwrap().eof),
            stderr_eof: self
                .output
                .as_ref()
                .map(|o| o.streams[1].lock().unwrap().eof),
        }
        .into_json()
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        if let Some(output) = &self.output {
            output.stop.store(true, Ordering::Release);
            for reader in &self.readers {
                os_api::agent::cancel_output_read(reader);
            }
        }
    }
}
#[derive(Default)]
pub(crate) struct ManagedRuns {
    runs: Mutex<VecDeque<Run>>,
    used: Arc<AtomicUsize>,
}
impl ManagedRuns {
    pub(crate) fn telemetry_targets(
        &self,
    ) -> Option<std::collections::HashMap<String, (u32, u64)>> {
        let runs = self.runs.try_lock().ok()?;
        Some(
            runs.iter()
                .map(|r| (r.id.clone(), (r.process.pid, r.process.creation_token)))
                .collect(),
        )
    }
    pub(crate) fn owns(&self, grant: &str, id: &str) -> bool {
        self.runs
            .try_lock()
            .is_ok_and(|runs| runs.iter().any(|r| r.grant == grant && r.id == id))
    }
    pub(crate) fn telemetry_count(&self) -> usize {
        self.runs.try_lock().map(|runs| runs.len()).unwrap_or(64)
    }
    pub(crate) fn launch(
        &self,
        grant: &str,
        name: String,
        spec: LaunchSpec,
        saved_rule: bool,
        register: impl FnOnce(&ManagedProcess) -> Result<(), String>,
    ) -> Result<Value, String> {
        let name = run_label(&name);
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "Managed run registry unavailable")?;
        let active = runs
            .iter()
            .filter(|r| r.process.exit_code().ok().flatten().is_none())
            .count();
        if active >= 16 {
            return Err("At most sixteen managed root processes may be active".into());
        }
        if spec.capture
            && runs
                .iter()
                .filter(|r| {
                    r.output
                        .as_ref()
                        .is_some_and(|_| !r.readers.iter().all(JoinHandle::is_finished))
                })
                .count()
                >= 4
        {
            return Err("At most four output captures may be draining".into());
        }
        while runs.len() >= 64 {
            let index = runs
                .iter()
                .position(|r| {
                    r.process.exit_code().ok().flatten().is_some()
                        && r.readers.iter().all(|reader| reader.is_finished())
                })
                .ok_or("Run history is full")?;
            runs.remove(index);
        }
        let id = os_api::agent::random_secret()?;
        let mut process = os_api::agent::launch_executable(&spec)?;
        let output = spec
            .capture
            .then(|| Arc::new(Output::new(self.used.clone())));
        let mut readers = Vec::new();
        for (index, file) in [process.stdout.take(), process.stderr.take()]
            .into_iter()
            .enumerate()
        {
            if let (Some(mut file), Some(output)) = (file, output.clone()) {
                let reader = std::thread::Builder::new()
                    .name(format!("managed-output-{index}"))
                    .stack_size(192 * 1024)
                    .spawn(move || {
                        let mut bytes = [0u8; 8192];
                        loop {
                            if output.stop.load(Ordering::Acquire) {
                                break;
                            }
                            match file.read(&mut bytes) {
                                Ok(0) => {
                                    output.streams[index].lock().unwrap().eof = true;
                                    break;
                                }
                                Ok(count) => output.append(index, &bytes[..count]),
                                Err(error) => {
                                    output.streams[index].lock().unwrap().error =
                                        Some(error.to_string());
                                    break;
                                }
                            }
                        }
                        output.streams[index].lock().unwrap().finished = true;
                    })
                    .map_err(|e| e.to_string())?;
                readers.push(reader);
            }
        }
        register(&process)?;
        process.resume()?;
        let run = Run {
            id,
            grant: grant.into(),
            name,
            process,
            output,
            readers,
            spec,
            saved_rule,
        };
        let view = run.view();
        runs.push_back(run);
        Ok(view)
    }
    pub(crate) fn list(&self, grant: &str) -> Value {
        let runs = self.runs.lock().unwrap();
        json!({"runs":runs.iter().filter(|run|run.grant==grant).map(Run::view).collect::<Vec<_>>(),"output_bytes":self.used.load(Ordering::Relaxed),"output_byte_limit":GLOBAL_BYTES})
    }
    pub(crate) fn stop(
        &self,
        grant: &str,
        id: &str,
        force: bool,
        allow_elevated: bool,
    ) -> Result<Value, String> {
        let runs = self.runs.lock().unwrap();
        let run = runs
            .iter()
            .find(|run| run.id == id && run.grant == grant)
            .ok_or("Run not retained for this client")?;
        if run.process.elevated && !allow_elevated {
            return Err("Administrator-process control was not granted".into());
        }
        if run.process.exit_code()?.is_some() {
            return Ok(run.view());
        }
        if force {
            run.process.terminate()?;
        } else if !run.process.request_close()? {
            return Err("No supported GUI close request exists for this root process; forced termination needs separate permission".into());
        }
        Ok(
            json!({"run_id":run.id,"pid":run.process.pid,"creation_token":run.process.creation_token.to_string(),"request":if force{"force_terminate"}else{"window_close"},"exit_code":run.process.exit_code()?}),
        )
    }
    pub(crate) fn restart_spec(
        &self,
        grant: &str,
        id: &str,
    ) -> Result<(String, LaunchSpec), String> {
        let runs = self.runs.lock().unwrap();
        let run = runs
            .iter()
            .find(|run| run.id == id && run.grant == grant)
            .ok_or("Run not retained for this client")?;
        if run.saved_rule {
            return Err(
                "Use restart_saved with the current saved rule revision and permissions".into(),
            );
        }
        let complete = run
            .output
            .as_ref()
            .is_none_or(|output| output.streams.iter().all(|s| s.lock().unwrap().eof));
        if !restart_ready(run.process.exit_code()?, complete) {
            return Err("Restart requires confirmed root exit and completed output drain; no implicit kill or duplicate launch was attempted".into());
        }
        Ok((run.name.clone(), run.spec.clone()))
    }
    pub(crate) fn output(
        &self,
        grant: &str,
        id: &str,
        stream: &str,
        after: u64,
        limit: usize,
    ) -> Result<Value, String> {
        if limit == 0 || limit > 16384 {
            return Err("Output limit must be 1..16384 bytes".into());
        }
        let runs = self.runs.lock().unwrap();
        let run = runs
            .iter()
            .find(|r| r.id == id && r.grant == grant)
            .ok_or("Run not retained for this client")?;
        let output = run
            .output
            .as_ref()
            .ok_or("Output capture was not requested for this run")?;
        output.page(
            match stream {
                "stdout" => 0,
                "stderr" => 1,
                _ => return Err("Choose stdout or stderr".into()),
            },
            after,
            limit,
        )
    }
}
fn restart_ready(exit: Option<u32>, capture_complete: bool) -> bool {
    exit.is_some() && capture_complete
}
#[cfg(test)]
pub(crate) fn maximum_run_list_fixture(text: &str, error: &str) -> Value {
    let label = run_label(text);
    let rows: Vec<_> = (0..64)
        .map(|_| {
            RunView {
                run_id: &"f".repeat(64),
                name: &label,
                pid: u32::MAX,
                creation_token: u64::MAX.to_string(),
                elevated: false,
                exit_code: Some(u32::MAX),
                root_exited: Some(false),
                exit_error: Some(bounded_display(error, 128, 256)),
                capture: false,
                capture_complete: false,
                stdout_eof: Some(false),
                stderr_eof: Some(false),
            }
            .into_json()
        })
        .collect();
    json!({"runs":rows,"output_bytes":GLOBAL_BYTES,"output_byte_limit":GLOBAL_BYTES})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_output_reservations_keep_the_global_byte_bound_and_release_it() {
        let used = Arc::new(AtomicUsize::new(0));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let used = used.clone();
                std::thread::spawn(move || {
                    (0..16)
                        .map(|_| {
                            let output = Output::new(used.clone());
                            output.append(0, &vec![b'x'; STREAM_BYTES]);
                            output
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let outputs: Vec<_> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect();
        let retained: usize = outputs
            .iter()
            .flat_map(|o| &o.streams)
            .map(|s| s.lock().unwrap().bytes.len())
            .sum();
        assert_eq!(retained, GLOBAL_BYTES);
        assert_eq!(used.load(Ordering::Relaxed), retained);
        drop(outputs);
        assert_eq!(used.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn managed_run_labels_bound_legacy_text_without_changing_identity_or_source() {
        let legacy = "😀".repeat(15000);
        let label = run_label(&legacy);
        assert_eq!(legacy.chars().count(), 15000);
        assert!(label.ends_with('…'));
        assert_eq!(label.chars().count(), 256);
        assert_eq!(run_label(&"😀".repeat(256)), "😀".repeat(256));
        assert_eq!(run_label("A program"), "A program");
        for text in ["\u{1}".repeat(15000), "\\\"".repeat(15000), legacy] {
            let list = maximum_run_list_fixture(&text, &text);
            for row in list["runs"].as_array().unwrap() {
                assert!(row["name"].as_str().unwrap().chars().count() <= 256);
                assert!(row["name"].to_string().len() <= 1026);
                assert!(row["exit_error"].to_string().len() <= 258);
                assert_eq!(row["run_id"], "f".repeat(64));
                assert_eq!(row["pid"], u32::MAX);
                assert_eq!(row["creation_token"], u64::MAX.to_string());
            }
        }
    }
    #[test]
    fn restart_needs_confirmed_exit_and_completed_capture() {
        assert!(!restart_ready(None, true));
        assert!(!restart_ready(Some(7), false));
        assert!(restart_ready(Some(7), true));
    }
    #[test]
    fn output_retains_bounded_suffix_with_explicit_gap_and_byte_cursors() {
        let used = Arc::new(AtomicUsize::new(0));
        let output = Output::new(used.clone());
        output.append(0, &vec![b'x'; STREAM_BYTES + 17]);
        let page = output.page(0, 0, 10).unwrap();
        assert_eq!(page["dropped_before"], "17");
        assert_eq!(page["next"], "27");
        assert_eq!(page["cursor_gap"], true);
        assert_eq!(used.load(Ordering::Relaxed), STREAM_BYTES);
        drop(output);
        assert_eq!(used.load(Ordering::Relaxed), 0);
    }
}
