# User-run render profiling

The separate Windows `render-profiling` build records reproductions for analysis
and comparison. Ordinary builds compile out the profiling calls and clock reads.
Measured CPU19 optimizations and remaining cold-frame, memory and measurement
limits are documented in [the performance probe report](cpu19-performance-probe.md).

## Running the Desktop package

1. Exit the ordinary CPU Affinity Tool, including any tray instance.
2. In **CPU Affinity Render Test**, run `Start-render-test.cmd` and accept UAC.
   Keep the diagnostic EXE and PS1 beside the CMD file.
3. Leave the window idle for about 20 seconds. Then exercise app/group graphs,
   All CPUs, Graphs/Processes, history hover, pin/unpin and table scrolling for
   1–2 minutes. Press **F8** immediately after a noticeable stutter.
4. Close the entire app using its native title-bar Close button. The package
   folder receives `render-log-*.zip` and `LAST-RESULT.txt`. Return that ZIP
   for analysis; it is never uploaded automatically.

Logging stops after five minutes or 64 MiB; the app continues running. An
explicit duration is supported: `./Start-render-test.ps1 -Seconds 180`
(10–900 seconds). Preparation errors appear in `TEST-ERROR.txt`.

The script makes an isolated sidecar copy of saved groups/rules, disables
autorun, affinity/priority enforcement, tuning and the startup Event Log query,
and enables CPU history with the saved retention. It never writes the original
configuration and checks its hash after the test. Test-window preference edits
affect only the copy. That copy contains application paths and **is excluded
from the ZIP**. The ordinary Desktop EXE is preserved.

## Log contents and interpretation

- `frame`: frame ID, layout pass, view/CPU/input-event count, inclusive stage
  wall time and scheduled CPU cycles, including mini/detail geometry rebuild
  counts. Backend timing explicitly belongs to the **previous** frame.
- `span`: background discovery/reconciliation and CPU-worker stages, on the
  same monotonic log clock.
- `resource`: process CPU over the actual interval (100% = one fully occupied
  logical CPU), private bytes, working set, process/writer cycles and drops.
- `history`: original interval time, pending/complete/lost transitions, known
  system-CPU count, pending length and retained `gap_reason` for unusable rows.
  Baseline, contribution capacity and stream/schema/order failures remain distinct;
  these categories describe observed failure boundaries, not guessed native causes.
- `telemetry`: observed process/thread counts, reserved history and categorized
  availability errors. Only successfully published snapshots enter this row.
- `note`: F8 markers and such conditions as a contended nonblocking history read.
- `session_start`, `renderer`, `session_end`: version, mask, priority, graphics
  renderer/driver description, bounds and explicit completion status.

`at_us` is monotonic time since session start. `wall_us` includes waits and
descheduling. `cycles` records scheduled thread work in **unscaled cycles**,
not nanoseconds. Nested stages include one another and must not be summed.
Graphics-driver, ETW and other work outside the instrumented stages remains
in the whole-process counters. A slow backend frame with cheap UI scopes
motivates a graphics/scheduling investigation, but this log alone cannot prove
GPU, driver, DPC or another process as its cause. Idle reactive-frame gaps are
normal and must not automatically be counted as stutters. OS-input-to-display
latency and GPU execution are not measured.

Measured threads never write files or wait for the logger queue. A fixed
256-record queue feeds one writer. Overflow is visible in `dropped_records`;
discarded queued records at bounded termination are counted in `queued_records`.
A missing completion marker means incomplete evidence. Writer serialization
and I/O have CPU cost: `writer_cycles` exposes that component, not all probe
overhead. No forced repaint, second scheduler trace or additional history ring
is introduced. The existing startup CPU19/BelowNormal policy remains in force.

## Build and validation

Build `cpu-affinity-tool` in release mode with `windows,render-profiling` and
`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`. The package retains the matching
PDB and EXE hash. `CPU_AFFINITY_RENDER_LOG` must explicitly name a new file even
in the diagnostic build; create-new refuses overwrite.
`CPU_AFFINITY_RENDER_SECONDS` bounds duration.

Unit/state tests cover frame/pass reset, saturating inclusive totals, unknown
cycles, nonblocking queue overflow/disconnection, valid JSONL size bounds,
write errors and a subprocess smoke with frames/background work/resources/
clean completion. `Start-render-test.ps1 -SelfTest` exercises safe configuration
selection, legacy missing autorun, defaults and immutable input without launch,
UAC or user-file access. The real native graphics scenario remains user-run;
the logger smoke does not substitute for that evidence.
