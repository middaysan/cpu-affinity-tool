# Debug-only CPU monitoring MCP

Build on Windows with:

```powershell
cargo build --features windows,debug-mcp,render-profiling --bin cpu-affinity-tool
```

This uses the existing optimized dev profile with debug assertions. Both the
feature and debug assertions are required. `cargo check --release --features
windows,debug-mcp --bin cpu-affinity-tool` must fail. The normal release has no
MCP module, listener, capture hooks or session handling.
The build script also rejects the release profile when debug assertions have
been manually enabled; changing that flag cannot admit MCP into release.

Place the dev EXE, matching PDB, `Start-mcp-debug.ps1`, `Start-mcp-debug.cmd` and
`cpu_affinity_mcp.py` in a dedicated folder. Start the launcher. It copies saved
groups into an isolated sidecar, disables autorun/enforcement/tuning, enables
CPU monitoring and requests elevation for read-only scheduler events. Original
settings are not edited. It creates a random per-launch key and local session
ticket; do not share that ticket or put it in source control.

Register a stdio MCP server in the local Codex configuration using Python and:

```text
cpu_affinity_mcp.py --session <package>/mcp-session.json
```

The adapter needs only Python's standard library. It reads the ticket for each
request, connects only to IPv4 loopback, disables proxies and redirects, and
prints only JSON-RPC on stdout. A Codex MCP catalog reload/restart may be needed
after adding the server. Launching the test app is separate from connecting the
adapter; normal application startup never opens this endpoint.

Use `cpu_state` to discover stable saved group/rule IDs and the current rendered
control rectangles. `cpu_open_graph`, `cpu_select`, `cpu_graph_options`,
`cpu_inspect` and `cpu_live` use production graph state. `cpu_table` pages the
actual selected-CPU contributor rows or current app-process rows. Exact sample
times, nulls and partial flags are preserved. `cpu_monitor` changes one isolated
monitoring preference per call. App-only process tables and group-only filters
retain their ordinary UI semantics.

`cpu_state` also exposes the existing `nav.groups` and `nav.tweaks` navigation
buttons for read-only page/refresh validation through `cpu_input`. No native
tweak action is registered.

`cpu_input` sends normalized pointer/click/wheel events to controls from the
current UI frame, without Windows input injection. Replies wait for the next
settled frame; re-read state after layout changes. `cpu_screenshot` returns a PNG
from the real renderer and can perturb that frame's performance. Screenshot
requests are marked in render-profiling logs. `cpu_close_test_app` performs normal
shutdown, including stopping the owned scheduler trace.

One blocking transport worker, four queued requests, bounded input/reply sizes
and eight-second GUI deadlines constrain the bridge. Graph/table serialization
occurs only on request. No second collector, history matrix, polling timer,
arbitrary file API, shell command, app launch or affinity/tweak action is exposed.
The session key protects this intentional elevated debug surface; this is not a
remote-management feature or an interface for untrusted clients.

Automated checks:

```powershell
cargo test --features windows,debug-mcp --bin cpu-affinity-tool debug_mcp
python scripts/test_cpu_affinity_mcp.py
powershell -NoProfile -File scripts/Start-mcp-debug.ps1 -SelfTest
```

The loopback integration test requires a host environment that permits local
sockets. Unit/state tests supplement an actual MCP/renderer smoke; neither
alone certifies every Windows scheduler sample or resolves prior CPU19 stalls.

## Diagnostic tools

Historical quality rows include `gap_reason` when an original interval was
unusable: startup baseline, tracked/contributor numeric capacity, event loss,
schema/order, buffer/clock/topology, missing frame or other known failure.
The same static category appears in optional render logs. It survives retention
resize and expires with its original row; current errors never replace it.
Per-PID measurement availability can still be unknown independently of delivery.

The server has fifteen tools. The three diagnostic additions are also entirely
debug-only. `debug-mcp` enables the existing `render-profiling` feature; recording
still requires its explicit environment opt-in, supplied by the test launcher.

- `cpu_performance(seconds=10)` reads a 1–60 second window from the existing
  profiler writer's bounded cache. It returns CPU relative to one logical CPU
  and the whole host, latest private/working memory from included resource
  observations, GUI frame percentiles and ten slowest frame IDs, and inclusive
  stage totals/maxima/cycles. GUI time adds only `gui_logic` and `gui_ui` and
  merges rendered passes with the same frame ID. Backend timing belongs to the
  preceding frame and remains separate. Stage aggregation uses one-second edge
  bins; reported window bounds are explicit. Resource averages use complete
  observed intervals only, so short windows can have null resource values.
  Stopped/not-started profiling, sample counts, data age, queue drops and frame
  eviction remain visible. No native poll, GUI wake or file read is performed
  by this request.
- `cpu_quality(offset=0, limit=50)` reads the retained history for the selected
  CPU/source, including the exact pinned/hovered row. Without a modal it uses
  system CPU0. Delivery counts and newest-first interval pages distinguish
  pending, completed and lost/unusable rows; load quality separately identifies
  complete, partial, unavailable or pre-source data. A completed interval does
  not prove complete per-CPU attribution. Known current collector errors are
  separate from historical state. Up to 100 incomplete current tracked exact
  PID/lifetime instances report CPU-time, CPU-mapping and name availability,
  deduplicated across owners. They are explicitly current data, not a historical
  roster or complete host coverage. The tool never invents a finer native cause
  that was not retained, or substitutes a nearby/current sample for a missing
  selected time.
- `cpu_wait(condition, timeout_ms=3000)` supports `new_sample`,
  `settings_applied`, `collection_released` and `ui_settled`, with 1–6000 ms
  limits. New sample means a completed original interval strictly newer than
  `after_seconds`, or the baseline captured when omitted; a lost/pending row
  does not satisfy it. Settings targets (`enabled`/`history_seconds`) require
  persisted preferences with cleared overrides and, while enabled, configured
  history buffers. Collection release requires the matching worker revision
  and no cleanup error. UI settlement means the next nondiscarded layout frame,
  not completion of every animation or GPU presentation. Results contain the
  observed state, effective baseline and elapsed time; normal timeout returns
  `satisfied=false, timed_out=true`. The GUI remains responsive. Only an explicit
  pending wait schedules checks at 100 ms, without repeatedly serializing graph
  history; such a diagnostic wait affects performance and is marked in the log.

The cache reserves 4096 compact frame records, 61 stage buckets and 64 resource
records once for the existing writer. The tested reservation is 290984 bytes.
No raw event log, new sampling worker or PID×CPU historical matrix is added.

The 2026-10-17 follow-up actual stdio test discovered all fifteen tools, read live
performance with zero logger drops, matched quality to the pinned Chrome
sample, resized retention and waited for its commit, confirmed release/history
empty, received an honest 256 ms timeout while Off, then resumed collection and
waited for a new completed interval. Normal shutdown returned zero and original
settings were unchanged. This was an interactive diagnostic smoke including a
renderer capture, not a controlled performance benchmark or a fix for the prior
CPU19 stalls.

## Local validation, 2026-10-17

The actual elevated optimized-dev app was exercised through stdio MCP on the
20-logical-CPU host, retaining CPU19 affinity and BelowNormal priority. Real egui
clicks opened the nested history selector without closing settings, selected
CPUs, pinned a sample, scrolled, changed the group filter and switched to the app
PID table. Tiles and contributor rows retained the same original sample while
the pointer left the graph. A positive Chrome CPU0 sample measured 3.8590846%;
its eleven process rows summed to 3.8590845%. Eight historical names were
explicitly unavailable; they were not replaced with current metadata. A pinned
host table had 55 rows and a verified nonzero internal scroll offset. Real PNG
frames were inspected. Monitoring Off was acknowledged by the worker after
trace cleanup and history release, and On resumed collection. Normal shutdown
returned exit code zero; original saved settings and the ordinary Desktop EXE
were unchanged.

The initial bridge mislabeled worker termination as collector disablement.
The corrected API distinguishes `worker_stopped` from
`disabled_acknowledged`; the latter requires the worker's matching release
revision, empty history and no cleanup error. Protocol, auth, input, frame
settlement, identity and acknowledgement regressions are covered automatically.

Two ten-second idle windows with graphs closed and the debug bridge plus render
profiler enabled recorded eight complete resource intervals each: On averaged
10.56% of one logical CPU, Off 10.08%. On private memory was 216.40–217.37 MiB
and working set 126.21–127.18 MiB; Off was 195.82–195.92 and 107.10–107.16 MiB.
The profiler reported no dropped records. These short debug/profile observations
do not isolate MCP's incremental cost, benchmark the shipping build, establish
universal gap-free attribution or resolve the earlier CPU19 performance findings.
