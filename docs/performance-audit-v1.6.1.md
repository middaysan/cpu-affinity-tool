# v1.6.1 performance and footprint proposal

Status: proposed; no runtime or dependency changes are implemented by this document.
Reviewed on 2026-09-15.

## Baseline and decision

Audit baseline: [`v1.6.1`](https://github.com/middaysan/cpu-affinity-tool/releases/tag/v1.6.1), peeled commit `68f1f7ccc8e2e9c755b3306bd88dda61b0157306`. The default branch was at that same commit during this review. The release was published on 2026-09-02. Stable artifacts are Windows-only; Linux remains a separate desktop beta.

The published EXE is **18,274,816 bytes (17.43 MiB)**. The separate PDB is **54,185,984 bytes (51.68 MiB)** and is a support artifact, not required to run the application. These are GitHub asset sizes, not memory measurements. Release metadata is available from the [GitHub release API](https://api.github.com/repos/middaysan/cpu-affinity-tool/releases/latest); the tag link above fixes the version under discussion.

**Recommendation: keep egui/eframe and the OS boundary. Trim enabled dependency features first, then remove repeated background and rendering work.** Forking the GUI framework or replacing it with custom Win32 UI is not justified by the evidence collected here.

Evidence consists of the release metadata, manifests and lockfiles, both entrypoints, execution monitors, process backend, GUI lifecycle/presenters, font setup, diagnostics and repository contracts. A binary inspection of the embedded ICO was also performed. No Windows runtime profile, Rust build, or before/after performance measurement was run in this audit environment: it has neither a Windows desktop nor an installed Rust toolchain. Findings below distinguish source-observed work from expected benefits that still require measurement.

## Prioritized work

Priority is implementation order, considering risk as well as potential benefit. It is not a measured ranking of memory consumers.

| Order | Change | Source-observed waste | Expected benefit | Risk / required gate |
| --- | --- | --- | --- | --- |
| 1 | Narrow `image` and Tokio features; remove unused direct dependencies | Generic image decoding and broad dependency activation | Smaller build graph and likely smaller EXE; RAM effect unmeasured | Low; both platform builds, icon decode and tray smoke |
| 2 | Explicit small Tokio worker pool | Both binaries use `Runtime::new()` for two permanent monitor tasks plus temporary corrections | Fewer worker threads and associated overhead on many-core machines | Medium; blocking OS calls can delay timers |
| 3 | Remove unconditional hidden-window repaint timer | Hidden shell requests another repaint after 250 ms | Fewer idle GUI wakeups | Medium; preserve all worker completion and IPC wake paths |
| 4 | Narrow monitor snapshots and skip empty discovery | Full storage/rule copies and unconditional process enumeration | Less periodic allocation and idle OS work | Medium; preserve cleanup and configuration changes |
| 5 | Eliminate repeated whole-system parent scans | A full Toolhelp snapshot per descendant parent query | Lower monitoring CPU with descendant-heavy workloads | High; process identity and parent validation must stay correct |
| 6 | Reduce Activity and picker work per frame | Whole-list formatting, filtering and layout | Lower allocation rate and faster interaction with long lists | Medium; wrapped rows and keyboard selection |
| 7 | Evaluate thin LTO and codegen settings | No explicit release tuning in root manifest | Potential EXE reduction; speed must be measured independently | Medium; build time, symbols and crash probes |
| Later | Native installed-app metadata discovery | PowerShell child process on catalog refresh / uncached package lookup | Potentially lower cold discovery latency and aggregate memory peak | High; preserve AppsFolder/AppX identity semantics |

## 1. Trim dependencies without trimming functionality

Evidence: [root manifest](../Cargo.toml), [OS manifest](../libs/os_api/Cargo.toml), [tray decoder](../src/tray.rs), [entrypoints](../src/main_windows.rs), [execution tasks](../src/app/features/execution/mod.rs).

The application requests `tokio` features `time`, `sync`, `rt`, `macros`, and `full`. Source uses task spawning, runtime entry, timers and asynchronous locks. It does not directly use Tokio networking, filesystem, process, signal or I/O APIs, or Tokio macros. An initial candidate is:

```toml
# Proposed replacement in [dependencies]; verify resolved features on both targets.
tokio = { version = "1.52.3", default-features = false, features = ["rt-multi-thread", "time", "sync"] }

# Proposed replacement in [target.'cfg(windows)'.dependencies].
image = { version = "0.25.10", default-features = false, features = ["ico"] }
```

`rt-multi-thread` remains necessary for the current runtime arrangement. Disabling `full` alone does not reduce the runtime's worker count; that is a separate change.

`image` currently enables its default formats and Rayon. Its direct application use is one bundled tray icon. The lockfile contains codec branches including `ravif`, `exr`, `tiff` and `image-webp`; inspect the resolved target graph before attributing their inclusion solely to this dependency. The [image feature documentation](https://docs.rs/crate/image/latest/features) describes the codec features and default activation.

The actual `assets/icon.ico` is **16,958 bytes**, with **one 64 x 64, 32-bit image**, whose payload begins with the 40-byte BMP/DIB header (`28 00 00 00`). It is not a PNG. The comment describing a 32 x 32 PNG and the helper name `decode_png_rgba` are misleading. Preserve the `ico` decoder and its BMP/PNG dependencies. Switching to PNG-only is a regression.

Use an explicitly selected ICO decoder (`load_from_memory_with_format(..., ImageFormat::Ico)`) and rename the helper when implementing this change. Return `RgbaImage::into_raw()` instead of `to_vec()` to transfer the decoded buffer rather than clone it. For this icon, the avoided RGBA copy is exactly **16,384 bytes once per tray initialization**, not a major steady-state RAM saving.

Source search found no direct usage of root Windows dependencies `parselnk`, `shlex`, or `libc`, or `ntapi` in `os_api`. Remove these declarations as separate small edits and regenerate both affected lockfiles with Cargo. Keep Linux `libc` and `shlex`: `libs/os_api/src/linux.rs` uses them. Windows shortcut handling already uses shell COM APIs in `windows/shell.rs`. An unused declaration can increase build work without contributing substantial linked code, so do not promise a matching EXE or RAM reduction.

The direct `winit` dependency is used for `raw_window_handle` imports in `shell/app.rs`; consider a direct `raw-window-handle` dependency instead, or disable unnecessary defaults if retaining `winit`. This removes an unnecessary direct coupling, not the transitive windowing library that eframe needs.

Cargo features are [additive and unified across dependency paths](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification). Another crate can re-enable a feature disabled at the root. A lockfile also includes multiple targets and optional resolution state; the presence of `wgpu`, GTK, Wayland or a codec in it does not prove that it is compiled into the Windows EXE.

Verification for this batch:

```text
cargo tree --locked --target x86_64-pc-windows-msvc --features windows -e features -i image
cargo tree --locked --target x86_64-pc-windows-msvc --features windows -e features -i tokio
cargo tree --locked --target x86_64-pc-windows-msvc --features windows -e normal,build
cargo tree --locked --target x86_64-unknown-linux-gnu --features linux -e normal,build
```

Capture these before and after lockfile regeneration. Add a Windows test decoding the real embedded icon through the production helper; check dimensions, RGBA buffer length and nonempty alpha, then smoke-test the visible tray icon. Run existing Windows and Linux CI gates and compare release EXE sizes under the same compiler.

## 2. Bound the runtime, with blocking work accounted for

Evidence: [Windows entrypoint](../src/main_windows.rs), [Linux entrypoint](../src/main_linux.rs), [monitor startup](../src/app/features/execution/mod.rs), [post-launch correction](../src/app/features/execution/launch.rs).

Both binaries construct `Runtime::new()` and enter it while eframe owns the main thread. Tokio's default multithreaded runtime sizes its async worker pool from available cores, unless overridden by configuration/environment. A **two-worker pool is an experiment worth testing**, not an established optimum. It should be created in one shared helper so platform entrypoints do not diverge:

```rust
// Proposed helper body; this is not an applied patch.
tokio::runtime::Builder::new_multi_thread()
    .worker_threads(2)
    .enable_time()
    .build()
```

Keep the runtime alive around the native event loop, and keep the entry guard. Do not replace it with `new_current_thread()` plus `enter()`: entering establishes context but does not drive that scheduler. See the upstream [runtime builder](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html) and [runtime entry semantics](https://docs.rs/tokio/latest/tokio/runtime/struct.Runtime.html#method.enter). Those links describe the runtime mechanism; the proposal does not upgrade the pinned Tokio version.

There are synchronous OS calls inside async monitoring/correction tasks, including package metadata lookup which can spawn PowerShell. A small pool can be blocked by those calls. Measure correction delay and monitor tick duration under multiple installed-app launches before choosing the pool size. Move slow operations into bounded, single-flight blocking work where necessary; do not replace a large worker pool with unlimited `spawn_blocking` submissions.

Tests must establish that a spawned timer progresses while the GUI/main test thread is outside `block_on`, that shutdown completes, and that slow injected discovery does not starve correction beyond the chosen latency budget. Test on a low-core machine and a many-core machine. Thread stack reservation is not equivalent to committed or resident RAM; count workers and measure Private Bytes separately.

## 3. Make the hidden shell event-driven

Evidence: `App::should_render` and `App::logic` in [shell/app.rs](../src/app/shell/app.rs), [monitor wake callbacks](../src/app/features/execution/monitor_events.rs), [tray events](../src/tray.rs), [picker refresh](../src/app/runtime/state.rs).

The hidden path unconditionally calls `request_repaint_after(250 ms)`. This schedules another GUI pass even when nothing changes: nominally four requested passes per second while hidden. It is not evidence of four full rendered frames or a measured CPU percentage; hidden UI rendering already returns early.

Tray events, monitor changes and local IPC already have explicit GUI wake callbacks. Remove the unconditional timer only after accounting for every asynchronous completion:

- Preserve crash-report and Event Log pending-work deadlines and their timeout behavior.
- Add a completion wake for installed-app catalog refresh, whose worker currently only sends into a channel. Otherwise hiding the window while refresh is in flight can leave completion unobserved until another event.
- Preserve delayed Event Log retry scheduling, monitor queue draining and terminal Quit behavior.

Test restore, quit, shortcut forwarding and refresh completion from an otherwise idle hidden window; include completion racing with hide/restore and a full monitor queue. The acceptance condition is no periodic shell deadline when hidden with no pending work, while actual events still wake the GUI promptly. Simply increasing 250 ms to several seconds hides unnecessary work and can introduce latency.

## 4. Narrow periodic snapshots and avoid empty work

Evidence: [tracking.rs](../src/app/features/execution/tracking.rs), [reconcile.rs](../src/app/features/execution/reconcile.rs), [rules snapshot](../src/app/features/rules/mod.rs), [central UI projection](../src/app/runtime/state.rs).

The discovery loop runs every two seconds and builds a process snapshot and name index even when `collect_configured_programs` returns no eligible matchers. The settings loop runs every three seconds and clones `AppStateStorage`, then builds another owned rule snapshot to extract a small settings map. `RulesContext::snapshot` clones each `AppToRun`. The central UI also builds that broad snapshot and then clones selected fields into its own projection.

Implement narrow, coherent projections while holding the storage read lock briefly, and release that lock before OS work. Keep logical group/rule identities and the runtime key contract. For settings, copy only key, owner IDs, display name, mask and priority; avoid retaining unrelated paths, CPU schema and editor data. For tracking, normalize a rule's tracked names once rather than twice. Build central-view data directly instead of constructing an intermediate full configuration copy.

For empty discovery, skip OS enumeration but still remove stale tracked entries, clean orphaned installed-package owner claims and notify the GUI when runtime state changes. For an empty running-app registry, avoid building settings data. An immediate fast return that bypasses cleanup is incorrect.

Do not skip discovery merely because automatic correction is disabled: observation and `SettingsMismatch` reporting remain useful. Keep immutable configuration snapshots coherent for one iteration. Revision-based caches are a later option; introduce them only if every rule mutation and identity change reliably invalidates them, since retained caches can increase RAM.

Consider explicitly using `MissedTickBehavior::Skip` after long blocking iterations instead of catching up every missed tick, provided tests preserve discovery/correction responsiveness. This is separate from changing the two- and three-second normal intervals.

Characterization/regression cases: no rules; no eligible path matchers; delete the last running rule; move or edit a rule during monitoring; monitoring disabled but mismatches visible; lock contention; and restored settings after the v1.6.1 protected-process failure path. Measure allocation bytes per iteration and OS snapshot counts, not just elapsed time.

## 5. Avoid a whole-system scan for each descendant

Evidence: `extend_with_descendants` in [tracking.rs](../src/app/features/execution/tracking.rs) and `OS::get_process_parent_pid` / `snapshot_process_tree_internal` in [Windows processes.rs](../libs/os_api/src/windows/processes.rs).

For each descendant candidate that reaches parent validation, the code calls `get_process_parent_pid`. On Windows, that function creates a new Toolhelp snapshot and reconstructs parent, child and process-name maps for the entire system. For `P` system processes and `D` candidate checks reaching this point, the extra enumeration work is approximately **O(D × P)** per iteration, in addition to initial discovery. This is conditional on descendants being enabled and such candidates existing; v10 defaults descendants off for new rules.

A batch parent-validation API is the candidate replacement. It should return only required relations and avoid repeated full name decoding/map construction. However, reusing the first discovery tree indiscriminately weakens the current fresh-parent check. Design and review a coherent validation phase: candidate token collection, fresh relation observation, parent/child instance revalidation, and conservative rejection of uncertain evidence. Do not keep relation caches across monitor ticks without an identity-aware invalidation design.

Preserve path/AUMID provenance, creation-token matching, child-created-after-parent checks, shared-package ownership and `manage_descendants`. Existing tests reject changed parent relationships, reused process instances and children predating a verified parent. Extend the fake OS to count snapshots and model process churn between validation phases. A reduction in snapshot calls is not sufficient unless these regression tests continue to reject stale evidence.

This is the strongest source-level algorithmic candidate for descendant-heavy workloads, but its semantic risk puts it after the simpler changes. Do not remove PID checks to make a benchmark faster.

## 6. Reduce rendering allocations and long-list layout

Evidence: [Activity presenter](../src/app/views/logs.rs), [LogManager](../src/app/models/log_manager.rs), [picker presenter](../src/app/views/installed_app_picker.rs), [picker snapshot construction](../src/app/runtime/state.rs).

Every Activity frame formats all chronological entries into owned strings, collects a vector, and lays out every row. The current formatter allocates a timestamp string and then a final line. Ordinary retention is 1,000 regular plus 200 important entries; sticky entries are outside those caps. Existing caps mean this is not proof of an unbounded regular-log leak.

There is a separate long-session memory risk: sticky deduplication uses the complete message string, while launch/save failures can include changing error details. Distinct failures can therefore accumulate without a count cap until Clear or exit. Reproduce this with many distinct injected failures. If it matters in practice, retain startup diagnostics separately and coalesce recurring error categories with the latest detail and a count, or define an explicit critical-history budget. That changes the documented sticky-retention policy and must update `AGENTS.md` with tests that preserve actionable diagnostics; silently truncating critical messages is not an acceptable optimization.

Start with borrowing entries and formatting only what is displayed, or use a revision-keyed presentation cache if its additional retained memory is acceptable. Avoid storing full duplicated raw and formatted histories purely to reduce CPU. Preserve newest-first ordering, Clear behavior, sticky deduplication and independent crash context. `LogManager::enforce_retention` also scans entries on insertion; optimize that only if event-storm profiles justify changing the representation and encapsulating the currently public `entries` field.

Virtualize large lists, but note that Activity labels wrap and therefore have variable heights. A naive fixed-height `ScrollArea::show_rows` conversion will clip or misplace them. Either maintain width/font-aware row heights with invalidation, use viewport-aware layout, or deliberately redesign collapsed rows with an explicit full-message view. The [egui scroll-area API](https://docs.rs/egui/0.35.0/egui/containers/scroll_area/struct.ScrollArea.html#method.show_rows) provides virtualization primitives, not automatic variable-height correctness.

The picker repeatedly filters the catalog and creates owned display rows. Cache normalized searchable strings and filtered indices by query/catalog generation; render only visible rows if layout permits. Preserve selection across refresh, arrow-key navigation, confirmation identity, long names, Unicode and Linux query-driven PATH discovery. Consider releasing an unusually large catalog after closing only if reopening latency and rediscovery memory are measured; retaining the cache is an intentional tradeoff.

Validate long multiline log messages, resizing/DPI changes, 1,200 ordinary logs plus sticky diagnostics, and a large synthetic catalog. Measure interactive frame time and allocations; these changes do not improve hidden-window rendering, which is already skipped.

## 7. Release configuration and framework boundaries

The root manifest already disables eframe defaults and explicitly selects `glow`, `accesskit`, `default_fonts`, `wayland` and `x11`. It does not explicitly select WGPU or eframe persistence. [eframe feature definitions](https://docs.rs/crate/eframe/0.35.0/features) explain those switches. Obtain the Windows target graph before claiming additional renderer removal.

Keep these distinctions when trimming the foundation:

- `accesskit` serves accessibility; removing it changes product behavior.
- `default_fonts` are used as fallbacks in `ui_font_definitions`; they are not unused simply because Inter is bundled. Test non-Latin names, symbols and all font families before altering this.
- `InterVariable.ttf` is 879,708 bytes. Its three configured weights use `FontData::from_static` over the same embedded bytes; three font registrations do not establish three copies of the TTF in the EXE. Rasterization/cache memory requires profiling.
- X11/Wayland support belongs to the Linux beta contract. Removing it globally to shrink a Windows executable is unjustified without target-graph evidence.
- `rfd`, tray integration, JSON persistence, OS process checks and crash diagnostics provide used behavior. Replacing them with custom implementations brings maintenance and compatibility costs.

Experiment with `[profile.release] lto = "thin"` and separately `codegen-units = 1`, keeping the compiler and dependency graph fixed. Measure size, build time, startup and runtime throughput. Do not assume `opt-level = "z"` is faster. See [Cargo release-profile controls](https://doc.rust-lang.org/cargo/reference/profiles.html#lto).

Retain `scripts/build-windows-release.ps1` and its line-table debug override. Keep matching EXE/PDB verification, the administrator manifest and crash probes. Do not change panic strategy or strip symbols as part of a footprint cleanup: they affect the diagnostics contract. PDB download size is a separate distribution concern from process RAM.

PowerShell-based AppsFolder and uncached package metadata lookups in [windows/shell.rs](../libs/os_api/src/windows/shell.rs) merit a separate cold-start/discovery experiment. Existing package metadata caching already reduces repeated work. Count child-process resource peaks as well as parent-process memory; a parent-only profile misses that cost. Native COM/WinRT replacement requires equivalent catalog filtering, AUMID identity, current-user package resolution and error handling.

## Measurement and acceptance plan

Use the same Windows machine, power plan, GPU driver, compiler, lockfile and test fixtures for baseline and candidate. Record the commit SHA, `rustc -Vv`, process count, logical CPU count and relevant environment overrides. Rebuild the baseline with the candidate compiler; a new build compared only to the published EXE confounds compiler changes with code changes.

| Scenario | Setup | Capture |
| --- | --- | --- |
| Cold launch | No existing GUI instance; isolated data directory; repeat 10 times | Time to first usable frame, peak Private Bytes, threads, handles |
| Hidden idle | No rules, then configured-but-stopped rules; let startup workers finish | Five-minute CPU-time delta, Private Bytes, private working set, GUI wake count |
| Normal monitoring | Known live path rules, correction on and off | Iteration duration, allocations, snapshot count, detection and correction delay |
| Descendant stress | Controlled process tree with descendants enabled; repeat with disabled | Whole-system snapshot count, CPU per iteration, stale-PID rejection |
| Installed targets | Cold and warm package cache; multiple launches and catalog refresh | Parent and PowerShell child peaks, launch correction delay, UI responsiveness |
| Interactive lists | Activity at ordinary retention caps and a large catalog | Frame-time distribution, allocation rate, scroll/selection correctness |
| Long session | Repeated open/close, refresh, launch/exit and hide/restore cycles | Private Bytes trend after warmup, thread/handle growth, functional smoke |

Private Bytes measures private committed memory; working set measures residency and is sensitive to OS trimming. Record both; do not infer a leak or a saving from a single Task Manager screenshot. CPU cost should include `TotalProcessorTime` delta divided by elapsed wall time; specify whether the percentage is normalized to one logical core or the full machine. Report medians and ranges over at least five paired runs for steady-state comparisons. Keep intentional performance thresholds separate from observed results.

Before production changes, add characterization tests as required by `AGENTS.md`. Every implementation batch must pass existing Windows and Linux fmt/clippy/tests/build checks and the applicable Windows manifest/PDB/crash gates. Manually verify tray restore/quit, saved-rule cold and forwarded launches, installed targets, persistence compatibility, and v1.6.1 operation-specific protected-process errors.

Acceptance requires a demonstrated improvement in the metric targeted by that batch, with no observed functional regressions and no unexplained worsening of monitoring/launch latency or steady-state memory outside run-to-run noise. Report raw measurements and the workload; do not set a universal claimed RAM reduction in advance.

## Suggested implementation sequence

1. Capture reproducible baselines and feature graphs; narrow dependency declarations and the icon decoder.
2. Test an explicit small runtime pool, isolate slow blocking operations as needed, and remove unnecessary hidden wakeups with completion coverage.
3. Introduce narrow projections and empty-work paths; then tackle descendant parent-scan batching with identity/race tests.
4. Optimize long-list presentation; evaluate release-profile settings independently.
5. Re-profile. Only then decide whether replacing PowerShell discovery or deeper framework changes has enough measurable benefit.

These are implementation batches, not modifications made by this proposal PR. Keeping them independently reviewable makes regressions attributable and allows a risky batch to be reverted without discarding verified improvements.
