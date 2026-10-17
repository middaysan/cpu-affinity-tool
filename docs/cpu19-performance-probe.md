# CPU19 performance probe — 2026-10-17

User-requested verification after startup self-affinity. The first probe below
is retained as historical evidence. The final section records the subsequent
MCP investigation, fixes and repeat measurements; neither run proves universally
loss-free telemetry or stutter-free presentation.

## Conditions

Optimized opt-in `native_cpu19_gui_cost_and_history`, wrapping the actual
production App, presenters and native CPU worker in an eframe/Glow window.
Verified mask `0x80000` (CPU19), BelowNormal, 20 logical CPUs. Four saved apps,
30-minute retention, 29 exact tracked instances and 3420 observed host threads
at the end. Sidecar was isolated; autorun, rule enforcement, tuning and Event
Log startup diagnostics were disabled. The real saved configuration was not
used for writes. The collector was stopped before the final assertions.

CPU is process CPU time / elapsed wall time, **without dividing by 20**:
100% means one logical CPU fully occupied. Resource means are weighted by
the measured interval duration. Each phase excludes its crossing interval.

The test adds preallocated timing vectors, one resource read and a repaint
deadline per second. Its absolute memory is for the instrumented optimized
test executable, not an independent measurement of the Desktop EXE. The
continuous-redraw phase is synthetic and is not normal reactive behavior.

## Results

| Phase | Measured seconds | CPU, one logical CPU | Private MiB | Working set MiB |
| --- | ---: | ---: | ---: | ---: |
| Monitoring Off | 18.21 | 11.50% | 210.11 | 121.71 |
| Monitoring On, main view idle | 24.05 | 10.65% | 221.82 | 146.02 |
| Interaction/main and graph | 78.92 | 13.72% | 224.69 | 150.43 |
| Graph, forced continuous redraw | 19.02 | 30.89% | 226.16 | 150.99 |

The cold Off phase was still growing its private allocation (198.44–218.54
MiB), so subtracting these phase means does not isolate collector overhead.
The final graph private allocation settled near 225.96 MiB. Settings estimated
24.5 MiB additional monitoring memory; reserved history was 15.70 MiB.
These observations do not prove an exact monitoring-only private delta.

Eframe previous-frame CPU/render elapsed time (excludes vsync wait):

| Phase | Frames | Median ms | p95 ms | Maximum ms |
| --- | ---: | ---: | ---: | ---: |
| On, main idle | 91 | 0.704 | 2.720 | 10.784 |
| Interaction | 1749 | 1.021 | 2.648 | 66.401 |
| Continuous redraw | 2535 | 1.101 | 1.283 | 57.533 |

Continuous-redraw frame spacing: median 6.958 ms, p95 7.458 ms,
maximum **131.414 ms**. Most frames were fast, but this contains an isolated
stall. UI presenter time p95 0.709 ms/max 32.472 ms; logic p95 0.0116 ms/max
3.890 ms. These are not an OS-input-to-display latency measurement and do not
identify whether a long frame came from our code, scheduling, graphics driver
or another host workload. No simultaneous unpinned comparison was run.

## History findings

After the warmup cutoff, the retained original rows contained:

- 129 completed intervals;
- **1 interval marked lost**;
- 3 latest pending intervals;
- **150 unavailable system CPU cells** within otherwise completed intervals;
- maximum pending count 3 and observed latest-finalized sample age 3.998 s;
- 91 fully attributed CPU/time comparisons with matching curve and row sums.

The observed diagnostic was `Waiting for tracked processes`. A pending tail
is delayed delivery, not proof of loss. The explicitly lost row and unknown
cells still prevent a no-gap acceptance. They are not counts of ETW events
lost. The current trace health path reports nonzero Windows loss counters as
errors; this probe did not export those native counters individually.

`max_worker_publication_gap_s=2.037` in the raw report is the largest step
between snapshots observed by the GUI's resource sampler. That sampler can
skip intermediate publications; it must not be interpreted as the worker's
actual maximum sampling interval.

The opt-in probe's zero-loss assertion failed, correctly preserving these
findings rather than declaring the run successful. Root cause and any repair
remain open; this run alone does not attribute them to CPU19 pinning.

## UI access and evidence

Computer Use captured the main window and Chrome graph with historical cursor
and contributor table. Automated input did not change the elevated window;
the user performed the graph interaction. Physical Escape then stopped
Computer Use, and no further UI automation was attempted. The already running
probe finished automatically at 165 seconds and exited after stopping its
collector. Further UI inspection was not performed.

Ignored local evidence: `target/cpu19-probe/{report.json,summary.json,gui.err,
gui.log,unit.log,fixture-evidence.json,done.txt}`. Counter characterization
passed (1 test). No production behavior was changed and the shipping binary
was not rebuilt for this measurement. The existing Desktop binary remains
SHA256 `F51EF6B9819CC5996554829593F7F6FF866FD6A7E6983296391A0AE2FC8AD19B`.

## MCP follow-up: fixes and repeat measurements

The subsequent investigation used the actual optimized-dev Desktop app through
the fifteen-tool stdio MCP adapter. The process retained CPU19 (`0x80000`) and
BelowNormal on the same 20-logical-CPU host, with four saved apps and 1800-second
history. MCP and profiling were enabled in both compared builds. Each scene
had a three-second warmup followed by twenty seconds of measurement. Input
scenes sent approximately ten requests per second; idle scenes did not use
`cpu_wait`, which deliberately requests UI checks while pending. Screenshots
were outside the measurement windows. The final log spans 408.83 seconds.

Changes preserve native recovery and action-time verification:

- Hidden or modal-covered System Tweaks cards no longer enumerate capabilities.
  The existing worker still publishes cheap health snapshots and performs owned
  inspections, reconciliation, expiry and recovery. Visible display cards share
  one inventory between observation and readiness instead of querying it three
  times. Opening the page admits an immediate observation.
- CPU modals retain the header and footer while skipping the underlying active
  page. Native eframe input and wheel behavior remain unchanged; no FPS limiter
  or input regulator was added.
- Original unusable intervals retain one bounded gap category in history, MCP
  quality and profiling. The first repeat run proved `contributor_capacity` at
  startup. Increasing the fixed reservation from 32 to 128 positive cells per
  CPU fixes the reproduced 800-cell burst; excess pressure still produces an
  honest gap. On this host the additional numeric reservation is about 0.70 MiB,
  included automatically in settings estimates and the existing admission budget.

CPU below is the **entire application's percentage of one logical CPU**, not
the whole-host percentage. Divide by twenty for this host's normalized value.

| Scene | Before | After | After GUI p95 / maximum, ms |
| --- | ---: | ---: | ---: |
| Monitoring Off, idle | 10.44–10.50% | 1.81–1.92% | 0.53 / 1.18 |
| Monitoring On, idle | 10.28–12.28% | 1.83–2.21% | 0.63 / 11.82 |
| Chrome graph, eight CPUs, idle | 12.15% | 4.06% | 1.11 / 3.59 |
| Chrome graph, twenty CPUs, idle | 12.23% | 2.66% | 1.56 / 3.30 |
| Chrome graph, hover | 24.24% | 10.97% | 0.84 / 1.47 |
| Chrome pinned, idle | 11.41% | 1.97% | 0.53 / 0.57 |
| App process table, scrolling | 35.19% | 26.12% | 0.71 / 15.30 |
| Host contributor table, scrolling | 37.29% | 25.91% | 1.29 / 8.09 |

The formerly dominant `tuning` thread fell from 9–10% to 0.47–0.78% of one
logical CPU in idle thread-time windows. Visible System Tweaks averaged 4.82%
for the whole application; returning to groups restored 2.16%. Continuous
scrolling still produces about 142–144 GUI frames per second through native
inertia, so the reduced CPU is not a claim of a frame-rate cap. Warm hover and
scroll outliers improved, but first-open/navigation frames reached 21.35–28.57
ms outside the warmed scene windows. Long warm table frames had few scheduled
cycles relative to wall time, consistent with waiting/descheduling; the profiler
does not uniquely identify each external scheduling or graphics cause.

The final quiet On sample used 218.66 MiB private memory and 127.71 MiB working
set; the paired Off sample used 199.05 / 109.44 MiB. Paired latest private
differences were 18.04–19.61 MiB. These include an instrumented process, warmed
UI caches and a changing host process inventory; they are not an exact allocation
ledger or a shipping-build memory benchmark. The settings estimate includes
configured maximum trace buffers and therefore need not equal this delta.

History recorded 355 completed original intervals and three unusable intervals,
all explicitly `baseline` from the initial start or deliberate restart. No new
event/schema/order/capacity failure was observed in the final run. Delayed
completion still filled the original row: median 2.00 s, p95 3.00 s, maximum
3.015 s. Six pending cuts at deliberate stops/exit were not classified as loss.

Delivery completion is separate from per-CPU measurement completeness. There
were **211 unavailable CPU cells across 70 completed intervals**, principally
after collection starts. Native host load requires a first scheduler switch on
that CPU at or before the interval begins; an unobserved boundary remains
unknown, even if the CPU appears idle or parked. One CPU remained unobserved
in 45 final post-restart intervals. These values were not changed to zero or
counted as complete coverage. Host process attribution can also remain partial.

Twelve exact pinned checks covered Chrome, ChatGPT, host and tracked-union
scopes. Graph, tile inspection and contributor table selected the same original
time. Complete contributor sums agreed within 0.0000012 percentage points;
one partial host row differed by 0.01008 points and retained its partial flag.
Pinned time survived scrolling. Off released history, On resumed collection,
and normal app shutdown returned zero with original saved settings unchanged.
The logger dropped and abandoned no records.

Final automated checks: ordinary app 639 passed, debug-MCP app 661, native
library 153 plus one doctest. App Clippy, native Clippy with the existing
unrelated baseline exclusions, formatting and diff checks passed. Ordinary
release marker absence and the release-feature rejection fence passed. The
800-cell regression failed before the capacity change and passed afterward.

The separate Desktop `CPU Affinity MCP Debug` EXE is SHA256
`DFB6FF2F322E0F0E68F87F56F9C70E0E1F344E9A8E2D6FAB2320492EE68B9293`,
with matching PDB. The ordinary Desktop EXE was preserved. Ignored local
evidence is `target/cpu19-after-{study.json,render.jsonl,analysis.json,
data-check.json,finish.json,exit.json}`; package metadata records the exact build.

## Focused scroll and cold-open investigation

The next user request was report-first investigation of the remaining roughly
26% scroll load and 28 ms opening frames. The existing Desktop debug EXE was
used without application code or build changes. Actual MCP inputs, independent
thread-time reads and original profiling records were captured in another
363.48-second isolated CPU19/BelowNormal session. Each resource scene was
twenty seconds after a three-second warmup. Thread and profiler windows differ
slightly; their CPU sums should not be forced equal.

| Scene | Process CPU, one logical CPU | Frames/s | Main thread CPU | AMD driver thread CPU |
| --- | ---: | ---: | ---: | ---: |
| App PID table, idle | 2.80% | 4.45 | 0.31% | 0.55% |
| App PID table, pointer movement at 10 Hz | 5.22% | 31.65 | 2.18% | 1.56% |
| App PID table, scroll events at 10 Hz | 23.82% | 144.45 | 11.62% | 9.36% |
| App PID table, scroll events near 30 Hz | 26.25% | 144.40 | 13.88% | 9.20% |
| Pinned host contributors, idle | 2.12% | 4.80 | 0.47% | 0.39% |
| Pinned host contributors, pointer movement at 10 Hz | 8.53% | 31.50 | 4.06% | 2.11% |
| Pinned host contributors, scroll events at 10 Hz | 26.99% | 143.00 | 13.96% | 8.50% |

Read-only elevated module inspection confirmed that the busy unnamed thread
entry addresses belong to `atio6axx.dll`; another graphics hook thread belongs
to `amdihk64.dll`. Their measured thread IDs were joined to the CPU windows.
These are graphics-driver-created threads inside the app process, and their
CPU is included in the process total on CPU19. Start-module attribution is not
a sampled stack of every instruction, nor proof that the driver is defective.
Non-elevated inspection returned access denied; the user approved only the
short read-only elevated helper. No driver settings were changed.

The scheduler consumer measured 0.39–0.62% in the two app-table scroll windows,
and 1.17% in the host-table window. The render-log writer measured 0.47–0.78%,
the MCP transport 0.16–0.94%, and the tuning worker 0.70–1.25%. These do not
explain the dominant scroll increment. RPC also has some GUI-thread work, so
these named-thread measurements do not isolate all diagnostic overhead.

The same input rate produced about 32 frames/s for pointer movement and about
144 for scroll. Local egui 0.36.2 `WheelState` accumulates wheel deltas and
applies a hard-coded exponential smoothing factor (90% over 0.1 seconds).
Pending wheel motion requests immediate repaints. The MCP injects real egui
Point/Move wheel events; its 10/30-point steps enter that smoothing path.
This is a controlled input test, not a physical mouse-to-display benchmark.

Individual warm app-table frames averaged 0.478 ms in the instrumented GUI
and 0.839 ms in the previous-frame eframe timing. Multiplying a small frame
cost by 144 explains substantial main-thread CPU, with additional driver work
on its own threads. Source inspection confirms that the selected Glow backend
tessellates and uploads draw meshes each painted frame. Its frame timer excludes
`swap_buffers`; backend timing is not a complete GPU/presentation measurement.

Row virtualization already exists in both tables. The remaining app-table draw
still formats PID, percentage and the CPU list and creates cell layouts on
each pass. The group modal also visits all twelve CPU tiles on every pass:
34,320 tile calls in 2,860 scroll frames, even when eight tiles were outside the
visible viewport. This tile stage consumed 549.8 ms of inclusive wall time over
the twenty-second window; contributor layout consumed 250.4 ms. Mini graphs
already skip invisible geometry, but tile label/frame layout does not. The full
window's header and footer are also rebuilt while only the lower table moves.

Cached history geometry is functioning: the large curve rebuilt 22 times in
2,860 frames (0.826 ms total rebuild wall time), mini geometry 88 times
(2.829 ms), and the overview copied 22 times. This is not a full history or
native measurement refresh on every scroll frame.

### Cold openings

First System Tweaks opening reproduced a 30.320 ms GUI frame; 30.218 ms was
inside Central and approximately 105 million scheduled thread cycles were
recorded. This was actual CPU work, unlike an outlier dominated by waiting.
Later opens typically used roughly 1 ms; a second cold content frame was
11.590 ms. The profiler captures the route before processing the header click,
so the first transition frame is labeled `groups` although Central draws Tweaks.

The first Chrome graph took 27.991 ms across two UI passes. The first pass's
modal used 24.490 ms; detail drawing used 7.135 ms, contributor drawing 4.643 ms,
and tile layout 3.011 ms. Detail geometry construction itself was just 0.047 ms;
the second pass's mini-geometry work added about 2.5 ms. The first PID-table
switch used 6.420 ms; repeated switches used about 1.25–1.29 ms.

An independent headless probe on CPU19 reused the actual shared Inter font
definitions/style, current egui dependencies and pixel scale 1.75. It measured
first and repeated text layout at the sizes used by the graph and Tweaks. In
two fresh-process repetitions, first use of sizes 9.5, 19, 12, 10, 11, 8 and 9
took 6.1–6.8 ms each; an already partly warm size 10.5 took about 3.5 ms.
Identical repeated layouts were below one microsecond at the probe's resolution.
Thus first-use glyph/size preparation is a demonstrated source of cold cost.
This probe is not an exact call-stack breakdown of all 28–30 ms in the app;
first layout and the necessary second pass also contribute.

### Candidate optimizations, not implemented

1. Prepare the actual required font sizes/glyphs before these screens are first
   opened, with a bounded startup/idle cost; consolidate nearly identical sizes.
   Reuse egui's caches rather than introducing a second text cache. Recheck DPI
   changes and previously unseen process-name characters. A preload must not
   simply turn several opening stalls into one long blocking startup frame.
2. Skip fully clipped tile rows while reserving their geometry, retain the
   existing table virtualization, and prepare stable display strings when the
   one-second process snapshot changes. Keep hover, selection, pin and original
   sample identities independent of these presentation caches.
3. The largest additional lever is reducing full-window paint demand during
   wheel smoothing. Explore a measured 60–90 Hz rendering budget or shorter
   smoothing while preserving responsive input and touchpad behavior. Native
   eframe 0.36.2 has no ready `max_fps` setting; ScrollArea `animated(false)` only
   governs scroll-to targets, not wheel smoothing. This requires an explicit
   design/validation step rather than assuming one existing flag solves it.
4. Reduce debug input replies to the data required by the caller. Currently an
   input request captures graph/table data and returns a full state snapshot
   after a settled frame. Keep full snapshots for explicit reads. The observed
   diagnostic overhead is secondary and cannot explain away 24–27% by itself.

Do not reduce telemetry frequency or remove process attribution to address this
rendering cost. A renderer-backend comparison can be a later experiment if the
driver cost remains high; a blind renderer/dependency switch is not supported
by this evidence.

The pinned sample survived scrolling. Final history had 361 completed intervals,
one initial baseline and two pending; no further unusable interval was recorded.
The logger dropped/abandoned zero records. App shutdown returned zero and the
original settings were unchanged. Private memory at the scene endpoints ranged
from 215.87 to 221.32 MiB, working set 125.03–130.30 MiB; these short windows do
not certify absence of leaks. The Desktop debug EXE and ordinary EXE hashes
remain exactly those recorded above. No application tests/builds were rerun for
this read-only investigation. Ignored evidence:
`target/cpu19-scroll-investigation{,-analysis,-finish,-exit}.json`,
`target/cpu19-scroll-investigation-render.jsonl`,
`target/cpu19-scroll-thread-origins.json` and `target/cpu19-font-probe*.log`.

## Scroll and cold-open implementation and repeat check

The user approved the first three optimizations. The Windows shell now prepares
the existing shared font sizes in batches of at most eight characters per
physical GUI frame, using egui's own caches. Preparation ends without a settled
repaint loop and restarts after a pixel-scale change. It paints no preload UI
and starts no worker. Unseen Unicode process names can still require first-use
glyph work. This moves bounded work into early frames; it does not eliminate
every initial layout cost.

CPU tiles reserve their measured geometry and skip fully clipped label/frame
layout. A renderer regression compares the actual row rectangles and requires
one visible layout instead of three. PID/load/core strings are prepared with
the existing current snapshot or exact-time contributor view, rather than
formatted on every paint. The current-process reservation accounts for its
new string capacities; sparse numeric history and its collector are unchanged.

While a CPU modal scrolls or drags, the existing native GUI thread observes a
90 Hz paint budget. A bounded sleep waits only for the remainder of one
11.12 ms interval before GUI work; clicks, keys and close events present in
the input bypass it. No input is removed, coalesced or rewritten. There is no
idle pacing timer, library fork or new scheduler. Telemetry remains at 1 Hz.
Previous-frame eframe wall timing includes this intentional wait, whereas the
instrumented GUI scopes start afterwards. The wait is not CPU execution, and
these two timings must not be presented as the same metric. Physical input
arriving during a wait is still subject to its bounded remainder.

The rebuilt optimized-dev/debug-MCP EXE is 18,675,200 bytes, SHA256
`14FAFA2E2F7E8ED215592724FF013825C774F63037421F290951E19C909DDD6A`;
the matching PDB GUID is `3b680b90-c19f-4b53-8e6a-d072935f72f4`, age 1.
The Desktop test package contains this pair. The ordinary Desktop EXE and the
unrelated native shell file retain their prior hashes. Ordinary release was
built and verified separately; all seven MCP/profiler marker checks and the
release feature rejection passed. The MCP implementation remains debug-only.

Actual fifteen-tool stdio MCP verification ran for 275.22 seconds on CPU19,
BelowNormal, twenty logical CPUs, four saved apps and 1,800-second retention.
Each scene below is twenty seconds after a three-second warmup. Baseline is
the preceding investigation, not a simultaneous paired process. Whole-process
CPU includes graphics and diagnostic threads, expressed relative to one
logical CPU. Input passes through the real egui handler; it is not a physical
mouse-to-display latency test.

| Scene | Previous CPU | New CPU | Previous frames/s | New frames/s | New GUI p95 / max, ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| App PID table, idle | 2.80% | 2.48% | 4.45 | 4.80 | 0.940 / 2.404 |
| App PID table, pointer at 10 Hz | 5.22% | 4.89% | 31.65 | 31.55 | 0.663 / 3.482 |
| App PID table, scroll at 10 Hz | 23.82% | 14.73% | 144.45 | 88.25 | 0.676 / 1.193 |
| App PID table, scroll near 30 Hz | 26.25% | 15.54% | 144.40 | 87.90 | 0.692 / 12.198 |
| Pinned host contributors, idle | 2.12% | 2.78% | 4.80 | 4.65 | 0.897 / 6.200 |
| Pinned host contributors, pointer at 10 Hz | 8.53% | 7.12% | 31.50 | 32.45 | 1.084 / 4.426 |
| Pinned host contributors, scroll at 10 Hz | 26.99% | 17.40% | 143.00 | 87.65 | 1.179 / 3.960 |

Scroll CPU fell by approximately 36–41%. The main thread measured 7.56–9.52%
across the three scroll scenes, compared with 11.62–13.96% previously. No old
process thread IDs were reused to attribute the new unnamed threads to modules.
The table near-30-Hz input loop delivered 528 requests in twenty seconds;
MCP's settled-frame request/reply latency is part of this diagnostic workload.

Pinned group scrolling laid out 14,024 tiles in 1,753 frames, eight per frame
including partially clipped rows, compared with twelve per frame previously.
Its tile stage used 240.2 ms inclusive wall time instead of 549.8 ms over the
twenty-second baseline window. Contributor layout used 163.0 ms versus 250.4 ms.
Curve geometry still rebuilt only 22 times (0.563 ms total), mini geometry 88
times (2.895 ms); pointer and wheel frames retain the original cache behavior.

The first Chrome graph frame fell from 27.991 to 12.561 ms across two UI passes.
The first System Tweaks opening fell from 30.320 to 22.017 ms, followed by a
9.601 ms content frame. Repeated Tweaks openings were 0.987–1.005 ms at the
transition; first PID-table opening was 4.952 ms versus 6.420 ms, repeated
switches 1.056–1.235 ms. Cold work therefore remains, particularly in Tweaks.
The initial startup frame was 26.574 ms. The scroll outlier at 12.198 ms had
only 2.85 million scheduled cycles, unlike the 22.017 ms cold frame's 76.85
million; this supports waiting/descheduling as a component without identifying
every stall's cause. No universal stutter-free claim is supported.

At the seven interaction-scene endpoints, private memory was 222.59–228.50 MiB
and working set 128.02–132.92 MiB, compared with 215.87–221.32 / 125.03–130.30
in the preceding investigation. The new caches/prepared glyphs have a retained
cost; these different short sessions do not isolate each allocation or prove
absence of leaks. A final quiet Off/On pair measured 204.04 / 224.67 MiB private
and 110.48 / 128.63 MiB working set: additional monitoring private memory was
20.63 MiB. Whole-process CPU was 1.65% Off and 2.22% On. These are instrumented
debug host observations, not a release benchmark or a guaranteed RAM estimate.

Twelve exact pinned comparisons cover Chrome CPU0, ChatGPT CPU19, host CPU0
and the tracked union CPU0. Plot, quality, tiles and contributor time identities
agree; complete app/union contribution sums differ by less than 0.0000006
percentage points. Host attribution remains explicitly partial (largest
observed subtotal difference 0.03084 points). The pin survives table scrolling.
Off confirms worker release and empty history; On records new original cuts.
The log records 247 completed intervals and two startup-baseline lost rows
across the two collection epochs, with no additional terminal loss category.
Pending cuts remain pending at shutdown; delivery completion is not proof
that every per-CPU scheduler boundary was observed. Ninety-three system CPU
cells in sixteen completed rows remained unknown; they are preserved as gaps.

The GUI suite passed 667 debug-MCP tests / six ignored and 645 ordinary tests /
five ignored. RED/GREEN coverage includes clipped geometry, snapshot text,
bounded preparation and discrete-input pacing bypass. Enabled/default strict
app Clippy, format, build, PDB identity and manifest checks passed. Native code
was unchanged by these three optimizations; its prior passing suite was not
repeated. Screenshots were inspected, logger drops/queued-at-exit were zero,
normal app exit returned zero and original user settings were preserved.
Ignored evidence is `target/cpu19-scroll-optimized{,-analysis,-data-check,
-finish,-exit}.json`, its `-render.jsonl` and screenshots. Desktop `BUILD.json`
and `CPU19-results.json` identify the exact measured package.
