#[cfg(test)]
use super::contributors::Contribution;
use super::contributors::{contributor_bytes, ContributorScope, ContributorView};
use super::history::{
    history_bytes, HistoricalSample, HistoryStore, LoadSource, Overview, SampleTime, Series,
};
use super::model::{normalized_instances, process_totals, GraphTarget, RuleTarget, Snapshot};
use super::worker_history::{EventHistory, OwnerInput};
use crate::app::models::{AppStateStorage, RunningApps};
use os_api::cpu_load::{CpuSampler, DetailedTrace, ProcessSampler};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock as TokioRwLock;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const TRACE_RETRY: Duration = Duration::from_secs(5);
const MAX_TRACKED_INSTANCES: usize = 1024;
// Allowance for the ETW consumer/runtime and one UI cache, apart from sized
// history/native buffers and observed tracking tables. This remains an estimate.
const ESTIMATED_COLLECTOR_RUNTIME_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct GraphData<'a> {
    pub series: &'a Series,
    pub overview: &'a Overview,
    pub sample: &'a HistoricalSample,
    pub hovered: Option<SampleTime>,
    pub contributors: &'a ContributorView,
    pub pending: bool,
    pub available: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Control {
    enabled: bool,
    revision: u64,
    history_seconds: u32,
    quit: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            enabled: false,
            revision: 0,
            history_seconds: 300,
            quit: false,
        }
    }
}

/// Latest control/current values are small. Full histories stay on the worker;
/// the GUI copies one full CPU series, a bounded overview and one hovered row
/// under short nonblocking reads. It never copies every full CPU history.
pub(crate) struct CpuLoadRuntime {
    pub(super) exports: Arc<super::export::ExportManager>,
    pub(super) export_ui: super::export_ui::ExportUi,
    control: Arc<(Mutex<Control>, Condvar)>,
    pub(super) published: Arc<RwLock<Arc<Snapshot>>>,
    pub(super) history: Arc<RwLock<HistoryStore>>,
    series_cache: Series,
    series_key: Option<(Option<RuleTarget>, usize)>,
    overview_cache: Overview,
    overview_key: Option<Option<RuleTarget>>,
    sample_cache: HistoricalSample,
    contributor_cache: ContributorView,
    contributor_key: Option<(Option<RuleTarget>, usize, SampleTime, u64)>,
    group_process_cache: Arc<super::process_list::ProcessList>,
    group_process_key: Option<(Vec<usize>, bool, Option<SampleTime>, u64)>,
    worker: Option<JoinHandle<()>>,
    running: Arc<TokioRwLock<RunningApps>>,
    source: Arc<RwLock<AppStateStorage>>,
    managed_runs: Arc<crate::app::features::execution::managed_runs::ManagedRuns>,
    pub graph: Option<GraphTarget>,
    pub selected_cpu: Option<usize>,
    pub show_all_cpus: bool,
    pub show_processes: bool,
    group_tracked_only: bool,
    pub(super) inspection: super::chart::Inspection,
    pub preference_override: Option<bool>,
    pub history_seconds_override: Option<u32>,
    pub preference_error: Option<String>,
    allow_worker: bool,
    stopped: Arc<AtomicBool>,
}
impl CpuLoadRuntime {
    pub fn new(
        running: Arc<TokioRwLock<RunningApps>>,
        allow_worker: bool,
        source: Arc<RwLock<AppStateStorage>>,
    ) -> Self {
        Self {
            exports: Default::default(),
            export_ui: Default::default(),
            control: Default::default(),
            published: Arc::new(RwLock::new(Arc::new(Snapshot::default()))),
            history: Default::default(),
            series_cache: Series::default(),
            series_key: None,
            overview_cache: Default::default(),
            overview_key: None,
            sample_cache: Default::default(),
            contributor_cache: Default::default(),
            contributor_key: None,
            group_process_cache: Default::default(),
            group_process_key: None,
            worker: None,
            running,
            source,
            managed_runs: Default::default(),
            graph: None,
            selected_cpu: None,
            show_all_cpus: false,
            show_processes: false,
            group_tracked_only: false,
            inspection: Default::default(),
            preference_override: None,
            history_seconds_override: None,
            preference_error: None,
            allow_worker,
            stopped: Arc::new(AtomicBool::new(true)),
        }
    }
    pub fn with_managed_runs(
        mut self,
        runs: Arc<crate::app::features::execution::managed_runs::ManagedRuns>,
    ) -> Self {
        self.managed_runs = runs;
        self
    }
    pub fn open(&mut self, target: GraphTarget) {
        self.set_tracked_only(matches!(target, GraphTarget::Group { .. }));
        self.show_all_cpus = matches!(target, GraphTarget::System);
        self.graph = Some(target);
        self.selected_cpu = None;
        self.show_processes = false;
        self.inspection = Default::default();
        self.overview_key = None;
        self.overview_cache.clear();
        self.sample_cache.clear();
        self.contributor_cache.clear();
        self.contributor_key = None;
        self.group_process_cache = Default::default();
        self.group_process_key = None;
    }
    pub fn close(&mut self) {
        self.set_tracked_only(false);
        self.graph = None;
        self.selected_cpu = None;
        self.series_cache = Series::default();
        self.series_key = None;
        self.overview_cache = Overview::default();
        self.overview_key = None;
        self.sample_cache = HistoricalSample::default();
        self.contributor_cache = ContributorView::default();
        self.contributor_key = None;
        self.group_process_cache = Default::default();
        self.group_process_key = None;
        self.show_all_cpus = false;
        self.show_processes = false;
        self.inspection = Default::default();
    }
    pub fn configure(&mut self, enabled: bool, _visible: bool) {
        if !enabled {
            self.close();
        }
        if let Ok(mut control) = self.control.0.lock() {
            if control.enabled != enabled {
                control.enabled = enabled;
                control.revision = control.revision.saturating_add(1);
                self.control.1.notify_one();
            }
        }
        if enabled && self.worker.is_none() && self.allow_worker {
            let control = self.control.clone();
            let published = self.published.clone();
            let history = self.history.clone();
            let running = self.running.clone();
            let source = self.source.clone();
            let managed_runs = self.managed_runs.clone();
            let stopped = self.stopped.clone();
            self.stopped.store(false, Ordering::Release);
            match std::thread::Builder::new()
                .name("cpu-load".into())
                .spawn(move || {
                    struct Finished(Arc<AtomicBool>);
                    impl Drop for Finished {
                        fn drop(&mut self) {
                            self.0.store(true, Ordering::Release);
                        }
                    }
                    let _finished = Finished(stopped);
                    run(control, published, history, running, source, managed_runs);
                }) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.stopped.store(true, Ordering::Release);
                    self.preference_error =
                        Some(format!("Could not start CPU monitoring: {error}"));
                }
            }
        }
    }
    pub fn set_history_seconds(&mut self, seconds: u32) {
        let seconds = AppStateStorage::normalize_cpu_load_history_seconds(seconds);
        if let Ok(mut control) = self.control.0.lock() {
            if control.history_seconds != seconds {
                control.history_seconds = seconds;
                self.control.1.notify_one();
            }
        }
    }
    pub(super) fn snapshot(&self) -> Arc<Snapshot> {
        let (revision, enabled) = self
            .control
            .0
            .lock()
            .map(|c| (c.revision, c.enabled))
            .unwrap_or((u64::MAX, true));
        self.published
            .try_read()
            .ok()
            .filter(|s| {
                s.revision == revision && (!enabled || s.at.elapsed() < Duration::from_secs(4))
            })
            .map(|s| s.clone())
            .unwrap_or_else(|| Arc::new(Snapshot::default()))
    }
    pub fn tracked_only(&self) -> bool {
        self.group_tracked_only
    }
    pub(super) fn tracked_group_load(&self, cpus: &[usize]) -> (Option<f32>, bool) {
        let Ok(history) = self.history.try_read() else {
            return (None, false);
        };
        let Some(time) = history.latest_finalized() else {
            return (None, false);
        };
        history.group_mean_at(LoadSource::Tracked, time, cpus)
    }
    pub fn set_tracked_only(&mut self, value: bool) {
        if self.group_tracked_only != value {
            self.group_tracked_only = value;
            self.series_key = None;
            self.overview_key = None;
            self.contributor_key = None;
        }
    }
    fn load_source<'a>(&self, target: Option<&'a RuleTarget>) -> LoadSource<'a> {
        match target {
            Some(target) => LoadSource::Rule(target),
            None if self.group_tracked_only => LoadSource::Tracked,
            None => LoadSource::System,
        }
    }
    pub(super) fn group_processes(
        &mut self,
        cpus: &[usize],
    ) -> Arc<super::process_list::ProcessList> {
        let Ok(history) = self.history.try_read() else {
            return Arc::new(super::process_list::ProcessList {
                group: true,
                ..Default::default()
            });
        };
        let time = history.latest_finalized();
        let same = self
            .group_process_key
            .as_ref()
            .is_some_and(|(mask, tracked, at, revision)| {
                mask == cpus
                    && *tracked == self.group_tracked_only
                    && *at == time
                    && *revision == history.revision
            });
        if !same {
            let list = if let Some(time) = time {
                let source = self.load_source(None);
                let known = cpus
                    .iter()
                    .filter(|&&cpu| history.value_at(source, time, cpu).is_some())
                    .count();
                history.contributors.process_list(
                    time,
                    cpus,
                    if self.group_tracked_only {
                        ContributorScope::Tracked
                    } else {
                        ContributorScope::System
                    },
                    known,
                )
            } else {
                super::process_list::ProcessList {
                    group: true,
                    ..Default::default()
                }
            };
            self.group_process_cache = Arc::new(list);
            self.group_process_key = Some((
                cpus.to_vec(),
                self.group_tracked_only,
                time,
                history.revision,
            ));
        }
        self.group_process_cache.clone()
    }
    #[cfg(any(test, all(debug_assertions, feature = "debug-mcp")))]
    pub(super) fn series(&mut self, target: Option<&RuleTarget>, cpu: usize) -> &Series {
        let same_key = self
            .series_key
            .as_ref()
            .is_some_and(|(key, previous_cpu)| key.as_ref() == target && *previous_cpu == cpu);
        if !same_key {
            // A contended copy may wait, but cannot relabel another series.
            self.series_cache.points.clear();
            self.series_cache.generation = self.series_cache.generation.wrapping_add(1);
        }
        if let Ok(history) = self.history.try_read() {
            if !same_key || self.series_cache.revision != history.revision {
                history.copy_series(self.load_source(target), cpu, &mut self.series_cache);
                self.series_key = Some((target.cloned(), cpu));
            }
        }
        &self.series_cache
    }
    /// Reuse the full selected series and a bounded all-CPU overview. Hover
    /// copies one exact row under a nonblocking read, never a full history grid.
    pub(super) fn graph_data(
        &mut self,
        target: Option<&RuleTarget>,
        cpu: usize,
        select: impl FnOnce(&Series) -> Option<SampleTime>,
    ) -> GraphData<'_> {
        crate::render_profile_scope!(GraphRead);
        // One read guard gives curve, tiles and contributors the same revision,
        // including when an older provisional row has just completed.
        let history_arc = self.history.clone();
        let guard = history_arc.try_read();
        let available = guard.is_ok();
        #[cfg(feature = "render-profiling")]
        if !available {
            crate::app::features::diagnostics::render_profile::note("history_read_contended");
        }
        let source = self.load_source(target);
        let same_series = self
            .series_key
            .as_ref()
            .is_some_and(|(key, previous_cpu)| key.as_ref() == target && *previous_cpu == cpu);
        if !same_series {
            self.series_cache.points.clear();
            self.series_cache.generation = self.series_cache.generation.wrapping_add(1);
        }
        if let Ok(history) = &guard {
            if !same_series || self.series_cache.revision != history.revision {
                crate::render_profile_scope!(SeriesCopy);
                history.copy_series(source, cpu, &mut self.series_cache);
                self.series_key = Some((target.cloned(), cpu));
            }
        }
        let hovered = select(&self.series_cache);
        self.sample_cache.clear();
        let same_overview = self
            .overview_key
            .as_ref()
            .is_some_and(|key| key.as_ref() == target);
        if !same_overview {
            self.overview_cache.clear();
        }
        let mut pending = false;
        if let Ok(history) = &guard {
            pending = history.pending_count() > 0;
            if !same_overview || self.overview_cache.revision != history.revision {
                crate::render_profile_scope!(OverviewCopy);
                history.copy_overview(source, &mut self.overview_cache);
                self.overview_key = Some(target.cloned());
            }
            let time = hovered.or_else(|| history.latest_finalized());
            if let Some(time) = time {
                history.copy_sample(source, time.seconds, &mut self.sample_cache);
                if self.sample_cache.time != Some(time) {
                    self.sample_cache.clear();
                }
            }
            let same = self.contributor_key.as_ref().is_some_and(
                |(key, previous_cpu, previous_time, revision)| {
                    key.as_ref() == target
                        && *previous_cpu == cpu
                        && Some(*previous_time) == time
                        && *revision == history.revision
                },
            );
            if !same {
                self.contributor_cache.clear();
                if let Some(time) = time {
                    if history.has_sample(source, time, cpu) {
                        history.contributors.copy(
                            time,
                            cpu,
                            match source {
                                LoadSource::System => ContributorScope::System,
                                LoadSource::Tracked => ContributorScope::Tracked,
                                LoadSource::Rule(target) => ContributorScope::Rule(&target.rule),
                                LoadSource::Run(id) => ContributorScope::Run(id),
                            },
                            &mut self.contributor_cache,
                        );
                        if history.value_at(source, time, cpu).is_none() {
                            self.contributor_cache.partial = true;
                            if self.contributor_cache.rows.is_empty() {
                                self.contributor_cache.available = false;
                            }
                        }
                    }
                }
                self.contributor_key =
                    time.map(|time| (target.cloned(), cpu, time, history.revision));
            }
        } else {
            self.contributor_cache.clear();
            self.contributor_key = None;
        }
        GraphData {
            series: &self.series_cache,
            overview: &self.overview_cache,
            sample: &self.sample_cache,
            hovered,
            contributors: &self.contributor_cache,
            pending,
            available,
        }
    }
    pub fn memory_estimate(&self, cpus: usize, rules: usize, seconds: u32) -> String {
        let runs = self.managed_runs.telemetry_count();
        let history = history_bytes(cpus, rules.saturating_add(runs), seconds)
            .and_then(|h| h.checked_add(contributor_bytes(cpus, seconds)?))
            .unwrap_or(usize::MAX);
        let snapshot = self.snapshot();
        let bookkeeping = ESTIMATED_COLLECTOR_RUNTIME_BYTES
            + snapshot.live_processes.reserved_bytes()
            + self.group_process_cache.reserved_bytes()
            + snapshot
                .bookkeeping_bytes
                .max(os_api::cpu_load::history_buffer_bytes(cpus).unwrap_or(usize::MAX))
            + snapshot.instances * (128 + cpus * 32)
            + snapshot.contributor_metadata_bytes
            + (rules + runs) * cpus * 24
            + cpus * 16;
        // Use the configured trace-buffer maximum for one conservative estimate.
        let trace_buffers = cpus.saturating_mul(2).max(4) * 2 * 64 * 1024;
        let mib = 1024.0 * 1024.0;
        format!(
            "Extra memory: ~{:.1} MiB\nHistory: {:.2} MiB · {cpus} logical CPUs · {rules} apps · {runs} managed roots\n{} tracked processes · {} observed threads{}\n{} retained process identities · bounded sparse contributor pool\nDelayed intervals: {:.2} MiB reserved · 16 max",
            (history as f64 + trace_buffers as f64 + bookkeeping as f64) / mib,
            history as f64 / mib,
            snapshot.instances,
            snapshot.threads,
            if snapshot.history_bytes > 0 {
                format!(
                    " · {:.2} MiB history reserved",
                    snapshot.history_bytes as f64 / mib
                )
            } else {
                String::new()
            },
            snapshot.contributor_identities,
            os_api::cpu_load::history_buffer_bytes(cpus).unwrap_or(usize::MAX) as f64 / mib
        )
    }
    pub fn shutdown(&mut self) {
        self.exports.cancelled.store(true, Ordering::Release);
        if let Ok(mut c) = self.control.0.lock() {
            c.quit = true;
            self.control.1.notify_one();
        }
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    pub(super) fn debug_disabled_acknowledged(&self) -> Option<bool> {
        let control = self.control.0.try_lock().ok()?;
        let published = self.published.try_read().ok()?;
        Some(
            !control.enabled
                && published.revision == control.revision
                && published.history_bytes == 0
                && published.error.is_none(),
        )
    }
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    pub(super) fn debug_wait_history(&self) -> Result<(Option<f64>, u32), String> {
        let history = self.history.try_read().map_err(|_| "History busy")?;
        Ok((history.debug_latest_completed(), history.debug_duration()))
    }
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    pub(super) fn debug_quality(
        &self,
        target: Option<&RuleTarget>,
        seconds: Option<f64>,
        cpu: usize,
        offset: usize,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        let history = self.history.try_read().map_err(|_| "History busy; retry")?;
        Ok(history.debug_quality(self.load_source(target), seconds, cpu, offset, limit))
    }
    pub fn status_error(&self) -> Option<String> {
        let snapshot = self.snapshot();
        snapshot
            .error
            .clone()
            .or_else(|| snapshot.detail_error.clone())
    }
}
impl Drop for CpuLoadRuntime {
    fn drop(&mut self) {
        self.shutdown();
        self.worker.take();
    }
}

struct TrackingSnapshot {
    generation: u64,
    targets: HashMap<RuleTarget, Vec<(u32, u64)>>,
}

fn collection_reset_required(current: &Control, previous: &Control) -> bool {
    // Off/on may be coalesced while a native read is finishing. Revisions
    // change only on enable/disable, so that boundary still clears history.
    !current.enabled || current.revision != previous.revision
}
fn tracked_instances(running: &Arc<TokioRwLock<RunningApps>>) -> Result<TrackingSnapshot, String> {
    let running = running
        .try_read()
        .map_err(|_| "Waiting for tracked processes")?;
    let generation = running
        .membership_generation()
        .ok_or("Tracked process membership is uncertain")?;
    let mut total = 0;
    let mut targets: HashMap<RuleTarget, Vec<(u32, u64)>> = HashMap::new();
    for app in running.apps.values() {
        let instances = normalized_instances(
            app.pids
                .iter()
                .filter_map(|pid| app.pid_instance_tokens.get(pid).map(|token| (*pid, *token))),
        );
        total += instances.len();
        if total > MAX_TRACKED_INSTANCES {
            return Err("Too many tracked processes for CPU monitoring".into());
        }
        if !instances.is_empty() {
            targets
                .entry(RuleTarget {
                    group: app.group_id.clone(),
                    rule: app.rule_id.clone(),
                })
                .or_default()
                .extend(instances);
        }
    }
    for instances in targets.values_mut() {
        instances.sort_unstable();
        instances.dedup();
    }
    Ok(TrackingSnapshot {
        generation,
        targets,
    })
}
fn inspect_process_identity(pid: u32, token: u64) -> Option<Arc<os_api::cpu_load::CpuProcess>> {
    let (path, actual) =
        crate::app::adapters::os::get_process_image_path_and_instance_token(pid).ok()?;
    if actual != token {
        return None;
    }
    let name = path
        .file_name()?
        .to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect::<String>();
    Some(Arc::new(os_api::cpu_load::CpuProcess {
        pid,
        instance: token,
        name: name.into(),
    }))
}
fn membership_matches(running: &Arc<TokioRwLock<RunningApps>>, generation: u64) -> bool {
    running
        .try_read()
        .is_ok_and(|current| current.membership_generation() == Some(generation))
}
fn saved_targets(
    source: &Arc<RwLock<AppStateStorage>>,
    targets: &mut Vec<RuleTarget>,
) -> Result<bool, String> {
    let state = source
        .try_read()
        .map_err(|_| "Waiting for saved CPU rules")?;
    let ids = state
        .rule_identities
        .as_ref()
        .ok_or("Saved CPU rule identities are unavailable")?;
    let iter = || {
        ids.groups.iter().zip(&state.groups).flat_map(|(g, saved)| {
            g.rule_ids
                .iter()
                .take(saved.programs.len())
                .map(move |r| (&g.id, r))
        })
    };
    if iter().eq(targets.iter().map(|t| (&t.group, &t.rule))) {
        return Ok(false);
    }
    *targets = iter()
        .map(|(g, r)| RuleTarget {
            group: g.clone(),
            rule: r.clone(),
        })
        .collect();
    Ok(true)
}
fn stop_trace(trace: &mut Option<DetailedTrace>, error: &mut Option<String>) {
    if let Some(mut active) = trace.take() {
        if let Err(e) = active.stop() {
            *error = Some(format!("Could not fully stop CPU monitoring: {e}"));
        }
    }
}
fn publish(
    control: &Arc<(Mutex<Control>, Condvar)>,
    current: &Control,
    published: &Arc<RwLock<Arc<Snapshot>>>,
    snapshot: &Snapshot,
) {
    if control.0.lock().is_ok_and(|c| *c == *current) {
        if let Ok(mut out) = published.write() {
            *out = Arc::new(snapshot.clone());
            #[cfg(feature = "render-profiling")]
            {
                let error = crate::app::features::diagnostics::render_profile::error_code(
                    snapshot
                        .detail_error
                        .as_deref()
                        .or(snapshot.error.as_deref()),
                );
                crate::app::features::diagnostics::render_profile::telemetry(
                    snapshot.instances,
                    snapshot.threads,
                    snapshot.history_bytes,
                    !matches!(error, "tracking_busy" | "membership_changed"),
                    error,
                );
            }
        }
    }
}

pub(super) fn aggregate_cpus(
    values: &mut [Option<f32>],
    partial: &mut [Option<f32>],
    instances: &[(u32, u64)],
    samples: &[os_api::cpu_load::ProcessCpuSample],
) {
    partial.fill(None);
    if instances.is_empty() {
        values.fill(None);
        return;
    }
    values.fill(Some(0.0));
    partial.fill(Some(0.0));
    for &instance in instances {
        let sample = samples
            .binary_search_by_key(&instance, |sample| (sample.pid, sample.token))
            .ok()
            .and_then(|index| samples.get(index));
        let Some(sample) = sample.filter(|sample| sample.cpus.len() == values.len()) else {
            values.fill(None);
            continue;
        };
        for ((value, measured), &next) in
            values.iter_mut().zip(partial.iter_mut()).zip(&sample.cpus)
        {
            let next = next.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 100.0));
            *value = value.and_then(|sum| next.map(|next| (sum + next).min(100.0)));
            if let Some(next) = next {
                *measured = measured.map(|sum| (sum + next).min(100.0));
            }
        }
    }
    for (full, measured) in values.iter().zip(partial.iter_mut()) {
        if full.is_some() || !measured.is_some_and(|v| v > 0.0) {
            *measured = None;
        }
    }
}

fn run(
    control: Arc<(Mutex<Control>, Condvar)>,
    published: Arc<RwLock<Arc<Snapshot>>>,
    history: Arc<RwLock<HistoryStore>>,
    running: Arc<TokioRwLock<RunningApps>>,
    source: Arc<RwLock<AppStateStorage>>,
    managed_runs: Arc<crate::app::features::execution::managed_runs::ManagedRuns>,
) {
    let mut next = Instant::now();
    let mut previous = Control::default();
    let mut cpu = None;
    let mut trace = None;
    let mut retry = Instant::now();
    let mut cleanup_error = None;
    let mut processes: HashMap<(u32, u64), ProcessSampler> = HashMap::new();
    let mut pid_totals: HashMap<(u32, u64), Option<f32>> = HashMap::new();
    let mut tracking: Option<TrackingSnapshot> = None;
    let mut targets = Vec::new();
    let mut run_targets = HashMap::new();
    let mut all = Vec::new();
    let mut snapshot = Snapshot::default();
    let mut configured_cpus = 0;
    let mut configuration_error = None;
    let mut event_history = EventHistory::default();
    let mut identities: HashMap<(u32, u64), Arc<os_api::cpu_load::CpuProcess>> = HashMap::new();
    let mut contributor_rules: HashMap<RuleTarget, Arc<crate::app::shared::ids::RuleId>> =
        HashMap::new();
    loop {
        let current = {
            let Ok(mut state) = control.0.lock() else {
                break;
            };
            while !state.quit && *state == previous && (!state.enabled || Instant::now() < next) {
                let wait = if state.enabled {
                    next.saturating_duration_since(Instant::now())
                } else {
                    Duration::from_secs(60)
                };
                match control.1.wait_timeout(state, wait) {
                    Ok((new, _)) => state = new,
                    Err(_) => return,
                }
            }
            state.clone()
        };
        if current.quit {
            break;
        }
        if collection_reset_required(&current, &previous) {
            stop_trace(&mut trace, &mut cleanup_error);
            cpu = None;
            processes.clear();
            pid_totals.clear();
            tracking = None;
            all.clear();
            targets.clear();
            run_targets.clear();
            event_history = EventHistory::default();
            identities.clear();
            contributor_rules.clear();
            if let Ok(mut store) = history.write() {
                *store = HistoryStore::default();
            }
            configured_cpus = 0;
            configuration_error = None;
            snapshot = Snapshot::default();
            snapshot.revision = current.revision;
            snapshot.error = cleanup_error.clone();
            if !current.enabled {
                publish(&control, &current, &published, &snapshot);
                previous = current;
                continue;
            }
        }
        let count = os_api::cpu_load::logical_cpu_count();
        let observed_runs = managed_runs
            .telemetry_targets()
            .unwrap_or_else(|| run_targets.clone());
        let runs_changed = observed_runs != run_targets;
        run_targets = observed_runs;
        let configuration_changed = match saved_targets(&source, &mut targets) {
            Ok(changed) => {
                changed
                    || count != configured_cpus
                    || current.history_seconds != previous.history_seconds
                    || !previous.enabled
            }
            Err(error) => {
                snapshot.error = Some(error);
                false
            }
        };
        if configuration_changed {
            if count != configured_cpus {
                cpu = None;
                processes.clear();
                if let Ok(mut store) = history.write() {
                    event_history.fail_with_reason(&mut store, super::history::GapReason::Topology);
                }
                stop_trace(&mut trace, &mut cleanup_error);
                retry = Instant::now();
            }
            let result = history
                .write()
                .map_err(|_| "CPU history lock failed".to_string())
                .and_then(|mut store| store.configure(count, current.history_seconds, &targets));
            configuration_error = result.err();
            configured_cpus = count;
            snapshot.process_cpus.retain(|t, _| targets.contains(t));
            snapshot
                .process_partial_cpus
                .retain(|t, _| targets.contains(t));
            snapshot.tracked_cpus.resize(count, None);
            snapshot.tracked_partial_cpus.resize(count, None);
            for target in &targets {
                contributor_rules
                    .entry(target.clone())
                    .or_insert_with(|| Arc::new(target.rule.clone()));
                snapshot
                    .process_cpus
                    .entry(target.clone())
                    .or_default()
                    .resize(count, None);
                snapshot
                    .process_partial_cpus
                    .entry(target.clone())
                    .or_default()
                    .resize(count, None);
            }
            contributor_rules.retain(|target, _| targets.contains(target));
        }
        if let Some(error) = &configuration_error {
            if let Ok(mut store) = history.write() {
                event_history
                    .fail_with_reason(&mut store, super::history::GapReason::Configuration);
            }
            stop_trace(&mut trace, &mut cleanup_error);
            snapshot.error = Some(error.clone());
            snapshot.cpus.clear();
            snapshot.processes.clear();
            snapshot.process_cpus.clear();
            snapshot.process_partial_cpus.clear();
            snapshot.run_cpus.clear();
            snapshot.run_partial_cpus.clear();
            snapshot.run_totals.clear();
            snapshot.scheduler_time = None;
            snapshot.tracked_cpus.clear();
            snapshot.tracked_partial_cpus.clear();
            snapshot.live_processes = Default::default();
            snapshot.revision = current.revision;
            snapshot.at = Instant::now();
            publish(&control, &current, &published, &snapshot);
            previous = current;
            next = Instant::now() + SAMPLE_INTERVAL;
            continue;
        }
        if configuration_changed || runs_changed {
            snapshot.run_error = history
                .write()
                .map_err(|_| "CPU history lock failed".to_string())
                .and_then(|mut store| {
                    store.configure_runs(&run_targets.keys().cloned().collect::<Vec<_>>())
                })
                .err();
        }
        // A retention-only wake reserves/resizes without adding an early sample.
        if current.enabled
            && previous.enabled
            && current.revision == previous.revision
            && Instant::now() < next
        {
            snapshot.history_bytes = history
                .read()
                .map(|h| h.reserved_bytes())
                .unwrap_or_default();
            publish(&control, &current, &published, &snapshot);
            previous = current;
            continue;
        }
        snapshot.error = None;
        crate::render_profile_scope!(CpuSampling);
        snapshot.detail_error = None;
        snapshot.collector_error = None;
        snapshot.parked = {
            crate::render_profile_scope!(Parking);
            os_api::cpu_load::parking_snapshot().unwrap_or_default()
        };
        {
            crate::render_profile_scope!(CpuPdh);
            match cpu.get_or_insert_with(CpuSampler::new) {
                Ok(sampler) => match sampler.sample() {
                    Ok(values) => snapshot.cpus = values,
                    Err(error) => {
                        snapshot.cpus.fill(None);
                        snapshot.error = Some(error);
                    }
                },
                Err(error) => {
                    snapshot.cpus.clear();
                    snapshot.error = Some(error.clone());
                }
            }
        }
        let unchanged = !configuration_changed
            && tracking
                .as_ref()
                .is_some_and(|t| membership_matches(&running, t.generation));
        if !unchanged {
            match tracked_instances(&running) {
                Ok(mut current) => {
                    // Deleted saved rules must release their native samplers,
                    // even before execution rediscovery publishes membership.
                    current.targets.retain(|target, _| targets.contains(target));
                    tracking = Some(current);
                }
                Err(error) => {
                    snapshot.detail_error = Some(error);
                }
            }
        }
        if configuration_changed || !unchanged || runs_changed {
            all.clear();
            if let Some(tracked) = &tracking {
                all.extend(
                    tracked
                        .targets
                        .iter()
                        .filter(|(target, _)| targets.contains(target))
                        .flat_map(|(_, instances)| instances.iter().copied()),
                );
            }
            all.extend(run_targets.values().copied());
            all.sort_unstable();
            all.dedup();
            let live: HashSet<_> = all.iter().copied().collect();
            processes.retain(|key, _| live.contains(key));
            identities.retain(|key, _| live.contains(key));
        }
        for values in snapshot.process_cpus.values_mut() {
            values.fill(None);
        }
        snapshot.tracked_cpus.fill(None);
        snapshot.tracked_partial_cpus.fill(None);
        for values in snapshot.process_partial_cpus.values_mut() {
            values.fill(None);
        }
        snapshot.processes.clear();
        pid_totals.clear();
        let tracked = tracking
            .as_ref()
            .filter(|t| membership_matches(&running, t.generation));
        {
            crate::render_profile_scope!(ProcessTimes);
            for &(pid, token) in &all {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    processes.entry((pid, token))
                {
                    if let Ok(p) = ProcessSampler::open(pid, token) {
                        entry.insert(p);
                    }
                }
                let value = processes
                    .get_mut(&(pid, token))
                    .and_then(|p| p.sample().ok().flatten());
                pid_totals.insert((pid, token), value);
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    identities.entry((pid, token))
                {
                    if let Some(identity) = inspect_process_identity(pid, token) {
                        entry.insert(identity);
                    }
                }
            }
            if let Some(tracked) = tracked {
                snapshot.processes = process_totals(&tracked.targets, |pid, token| {
                    pid_totals.get(&(pid, token)).copied().flatten()
                });
            }
            snapshot.run_totals = run_targets
                .iter()
                .map(|(id, instance)| (id.clone(), pid_totals.get(instance).copied().flatten()))
                .collect();
        }
        let mut live_processes = tracking
            .as_ref()
            .map(|tracked| {
                super::process_list::ProcessList::from_tracking(
                    &tracked.targets,
                    &pid_totals,
                    &identities,
                )
            })
            .unwrap_or_default();
        if tracked.is_none() {
            live_processes.invalidate();
        }
        let mut trace_error = None;
        if trace.is_none() && Instant::now() >= retry {
            match DetailedTrace::start_history() {
                Ok(active) => trace = Some(active),
                Err(error) => {
                    trace_error = Some(error);
                    retry = Instant::now() + TRACE_RETRY;
                }
            }
        }
        if let Some(active) = trace.as_mut() {
            let frames = {
                crate::render_profile_scope!(TraceDrain);
                active.history_frames()
            };
            match frames {
                Ok(frames) => {
                    if let Ok(mut store) = history.write() {
                        for frame in frames {
                            event_history.apply(frame, &mut store);
                        }
                    } else {
                        trace_error = Some("CPU history lock failed".into());
                    }
                }
                Err(error) => trace_error = Some(error),
            }
            if trace_error.is_none() {
                for &(pid, token) in &all {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        identities.entry((pid, token))
                    {
                        if let Some(identity) = active.process_identity(pid, token) {
                            entry.insert(identity);
                        }
                    }
                }
                let valid =
                    tracked.filter(|tracked| membership_matches(&running, tracked.generation));
                let tick = {
                    crate::render_profile_scope!(TraceTick);
                    active.history_tick(if valid.is_some() {
                        &all
                    } else {
                        all.clear();
                        all.extend(run_targets.values().copied());
                        all.sort_unstable();
                        all.dedup();
                        &all
                    })
                };
                match tick {
                    Ok(tick) => {
                        if let Ok(mut store) = history.write() {
                            let input = OwnerInput {
                                generation: valid.map(|t| t.generation),
                                targets: valid.map(|t| &t.targets),
                                names: &identities,
                                rules: &contributor_rules,
                                runs: &run_targets,
                                changed: configuration_changed,
                            };
                            if let Err(error) =
                                event_history.enqueue(tick, input, &snapshot.parked, &mut store)
                            {
                                trace_error = Some(error);
                            }
                        } else {
                            trace_error = Some("CPU history lock failed".into());
                        }
                    }
                    Err(error) => trace_error = Some(error),
                }
            }
            (snapshot.bookkeeping_bytes, snapshot.threads) = active.memory_usage();
            snapshot.bookkeeping_bytes += event_history.bookkeeping_bytes();
        }
        if let Some(error) = trace_error {
            let gap_reason = super::history::GapReason::from_error(&error);
            snapshot.collector_error = Some(error.clone());
            snapshot.detail_error = Some(error);
            if let Ok(mut store) = history.write() {
                event_history.fail_with_reason(&mut store, gap_reason);
            }
            stop_trace(&mut trace, &mut cleanup_error);
            retry = Instant::now() + TRACE_RETRY;
        }
        if trace.is_none() {
            if let Ok((seconds, wall)) = os_api::cpu_load::history_clock() {
                if let Ok(mut store) = history.write() {
                    let time = SampleTime { seconds, wall };
                    store.push_pending(time, &snapshot.parked);
                    store.mark_lost_reason(
                        time,
                        snapshot
                            .detail_error
                            .as_deref()
                            .map(super::history::GapReason::from_error)
                            .unwrap_or(super::history::GapReason::TraceUnavailable),
                    );
                    store.contributors.push(time, &[], false);
                }
            }
        }
        event_history.publish(&mut snapshot);
        live_processes.record_cores(count, event_history.samples());
        if tracked.is_none_or(|tracked| !membership_matches(&running, tracked.generation)) {
            snapshot.processes.clear();
            live_processes.invalidate();
            snapshot.detail_error.get_or_insert_with(|| {
                "Tracked processes changed; waiting for the next interval".into()
            });
        }
        if trace.is_none() {
            let error = snapshot
                .collector_error
                .get_or_insert_with(|| "Waiting to restart detailed CPU monitoring".into());
            snapshot.detail_error.get_or_insert_with(|| error.clone());
        }
        snapshot.instances = all.len();
        live_processes.time = Some(SystemTime::now());
        snapshot.live_processes = Arc::new(live_processes);
        if let Ok(store) = history.read() {
            snapshot.scheduler_time = store.latest_finalized();
            snapshot.history_bytes = store.reserved_bytes();
            snapshot.contributor_metadata_bytes = store.contributors.metadata_bytes();
            snapshot.contributor_identities = store.contributors.metadata_count();
        }
        snapshot.revision = current.revision;
        snapshot.at = Instant::now();
        if cleanup_error.is_some() {
            snapshot.error = cleanup_error.clone();
        }
        publish(&control, &current, &published, &snapshot);
        previous = current;
        next += SAMPLE_INTERVAL;
        if next <= Instant::now() {
            next = Instant::now() + SAMPLE_INTERVAL;
        }
    }
    stop_trace(&mut trace, &mut cleanup_error);
    if let Ok(mut store) = history.write() {
        *store = HistoryStore::default();
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "graph_tests.rs"]
mod graph_tests;

#[cfg(test)]
#[path = "native_gui_probe.rs"]
mod native_gui_probe;
