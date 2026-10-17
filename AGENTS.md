# AGENTS.md for `cpu-affinity-tool`

## Purpose
This file records the actual repository structure, platform boundaries, runtime architecture, build and release contract, and the repo-specific workflow rules that must stay true for this project.

Keep it truthful. If architecture, CI, release flow, platform scope, or important repository structure changes, update `AGENTS.md` in the same change.

## Repo workflow contract
This repository uses the staged workflow standard with this file as the canonical repo contract.

Workflow facts:
- canonical repo contract: root `AGENTS.md`
- optional local overlay: `.codex/AGENTS.md`
- canonical local stage artifact: `.codex/ROADMAP.md`
- optional local task handoff workspace: ignored `tasks/`
- local user-facing roadmap content may be written in Russian
- repo `workflow_mode`: `staged-default`

Overlay rules:
- `.codex/AGENTS.md` may exist only as a local additive overlay
- it may not contradict facts or restrictions from this file
- it may only tighten workflow activation through `workflow_override: inherit | explicit-only`
- it may not weaken repo-shared policy

Task handoff rules:
- `tasks/` is a local-only, git-ignored workspace for investigation context, scope, user decisions, implementation plans, and cross-session handoff notes
- `tasks/` supplements but never replaces or contradicts root `AGENTS.md` or `.codex/ROADMAP.md`
- `.codex/ROADMAP.md` remains the only canonical owner of stage identity, order, and status
- at session start, after reading this file and the optional overlay and roadmap, read `tasks/README.md` when it exists and then the relevant task's `HANDOFF.md`
- task directory names use descriptive lowercase kebab-case; each active task keeps `HANDOFF.md`, `CONTEXT.md`, `SCOPE.md`, `DECISIONS.md`, and `PLAN.md`
- update the task handoff after material discoveries, user decisions, scope changes, implementation checkpoints, or new blockers
- do not store secrets, credentials, personal data, or generated build artifacts in `tasks/`

Roadmap identity rules:
- stages use immutable `stage_id` values such as `S00`, `S01`, `S02`
- display order numbers are convenience only
- once `.codex/ROADMAP.md` records a roadmap mutation, `stage_id` becomes the canonical stage reference
- legacy root `ROADMAP.md` and `ROADMAP_PROMPTS.md` may exist as ignored local convenience files, but they are not canonical workflow artifacts

Freshness rules:
- root `AGENTS.md` is always reread at session start and on `Status`
- if assistant detects drift between this file and repo reality, it must flag the conflicting section and carry the tracked update in the next relevant repo change
- `.codex/ROADMAP.md` owns only stages, statuses, deferred items, residual risks, freshness metadata, and append-only roadmap-change history
- `tasks/` may summarize roadmap state for handoff convenience, but any conflict is resolved in favor of `.codex/ROADMAP.md`

Test-first development rules:
- Behavior changes must follow TDD: write or update the failing or characterization test for the desired behavior before implementing the production change whenever technically possible
- Regression fixes must include a regression test that would have failed on the broken behavior before the fix
- UI and runtime changes should push decision logic into pure helpers or state-level methods so behavior can be tested without relying only on manual GUI interaction
- If an OS or GUI interaction cannot be reproduced deterministically in automated tests, add the closest reliable unit or state-level coverage and document the required manual smoke validation in the change plan or release notes
- Behavior changes must not rely on manual testing alone unless automated coverage is impractical and that limitation is explicitly called out

## Project and platform status
`cpu-affinity-tool` is a desktop utility for managing CPU affinity and process priority.

Repository binaries:
- `cpu-affinity-tool` - primary Windows binary
- `cpu-affinity-tool-linux` - feature-gated Linux entrypoint

Current platform reality:
- Windows is the primary released and explicitly supported platform
- Linux code exists as a CI build/test/clippy validated desktop beta path from source for `x86_64` `glibc`; desktop sessions on `X11` or `Wayland` are covered by manual beta smoke validation
- Linux also has a separate beta prerelease artifact contract under `linux-beta-v*` tags, but it is not part of the stable release contract
- System Tweaks is Windows-only. Linux has no tuning route or rule-profile controls, no registered modules, recovery lease, native module actions or execution/foreground observations. The existing background queue remains only for ordinary state/rule saves; persisted tuning data is retained without interpretation or association edits.
- the project must not be described as a fully cross-platform desktop app

## Repository map
Key directories:
- `src/` - application runtime code and entrypoints
- `src/app/shell/` - the top-level `eframe::App` shell, route enums, transient UI sessions, typed shell events, and presenter module ownership
- `src/app/shell/render_preparation.rs` - bounded Windows font preparation and the CPU-modal scroll paint budget on the existing GUI thread
- `src/app/features/` - bounded feature modules for `rules`, `execution`, `preferences`, `shortcut`, `topology`, `diagnostics`, `cpu_load`, and `system_tweaks`
- `src/app/features/cpu_load/` - Windows-only read-only CPU telemetry, bounded histories, a single sampling worker and the compact group/process graph presenter. Native PDH/process-time/ETW collectors live under `libs/os_api/src/windows/cpu_load/` behind `libs/os_api/src/cpu_load.rs`.
- `src/app/features/system_tweaks/platform/` - small typed contracts/codecs, the module registry and owned-field controller
- `src/app/features/system_tweaks/config.rs` - bounded profile/configuration persistence shapes and exact forward-data retention; parsing supplies no module execution or native source authority
- `src/app/features/system_tweaks/service.rs` - candidate-state saves with logical identities and manual module requests
- `src/app/features/system_tweaks/execution_bridge.rs` - bounded execution/foreground observations with copy/inspect/recheck publication, scheduled refresh and coalesced refresh requests
- `src/app/features/system_tweaks/session_reducer.rs` - one bounded application worker, existing execution observations and immutable profile/card/shutdown snapshots
- `src/app/features/system_tweaks/presentation.rs` - typed module validation/tweak metadata and transient help/undo sessions; `src/app/features/system_tweaks/view.rs` is the compact System Tweaks presenter
- The existing `tweaks/power_mode/` adapter applies only the AC power-mode preference for new tweaks. Retired CPU configurations and their exact scheme/setting bindings remain readable for recovery and Undo. `libs/os_api/src/gaming_cpu_power.rs` and `libs/os_api/src/windows/gaming/cpu_power.rs` retain their typed native restoration boundary, including exact absence restoration.
- `src/app/features/system_tweaks/pc_checks.rs` - pure summaries, hardware guidance and explicit read-only PC checks; bounded native reads live in `libs/os_api/src/windows/pc_checks.rs`
- `src/app/adapters/` - seams for persisted state loading, OS helpers, and installed-app discovery
- `src/app/runtime/` - thin composition-root state facade kept around `AppState`
- `src/app/models/` - persisted schema, domain and runtime-independent data types, CPU preset and meta helpers, `LogManager`, and running-app tracking structures
- `src/app/models/app_state_storage/` - internal persistence modules for state path resolution, storage I/O, migrations, and schema refresh; `app_state_storage.rs` remains the public storage schema and API entrypoint
- `libs/os_api/` - platform boundary for OS-specific operations; Windows internals are split under `libs/os_api/src/windows/`, including startup-only own-process scheduling in `self_affinity.rs` and suspended-launch affinity inheritance handling in `launch.rs`; Linux remains a single-file desktop beta backend
- `libs/os_api/src/windows/file_drop.rs` - GUI-thread Shell file-drop registration for the Windows root, with window-scoped UIPI exceptions, bounded path-only payloads and captured physical release coordinates; no worker or process-wide filter
- `assets/` - icon, screenshot, bundled Inter UI font and license, `cpu_presets.json`, and social-preview guidance
- `design/` - standalone HTML design reference for the approved compact Inter-based interface; it loads only repository-local assets and is not runtime input
- `docs/` - release/process documentation and user-facing comparison/rationale references
- `scripts/` - committed release/build verification helper scripts
- `vendor/eframe/` - published eframe 0.36.2 with one documented initial-root-visibility fix; upstream provenance, licenses and removal criteria are in `LOCAL-PATCH.md`
- `.github/workflows/` - CI and GitHub Release automation
- `changelogs/` - manual release notes
- `tasks/` - optional ignored local workspace for task dossiers and cross-session handoff

Important root files:
- `Cargo.toml` - package metadata, binaries, features, dependencies
- `LICENSE` - MIT license
- `build.rs` - Windows resource embedding and rebuild hooks
- `app.manifest` - embedded Windows release manifest with elevated privilege model
- `Makefile.toml` - local developer automation wrapper
- `README.md` - user-facing project description
- `CHANGELOG.md` - consolidated human-facing release history
- `CONTRIBUTING.md` - contribution workflow and review expectations
- `SECURITY.md` - private security reporting policy
- `SUPPORT.md` - support routing and diagnostics expectations
- `.github/ISSUE_TEMPLATE/` - structured issue intake for bugs and feature requests
- `docs/comparison.md` - comparison with Task Manager, Process Lasso, and CLI workflows
- `docs/why.md` - rationale, limits, and non-goals of affinity management
- `docs/release-checklist.md` - manual checklist for the current Windows-only release contract
- `docs/linux-beta-release-checklist.md` - manual checklist for the Linux beta prerelease contract
- `docs/release-process.md` - current tag-based stable Windows release flow plus Linux beta prerelease flow and release-notes template
- `docs/release-smoke-matrix.md` - compact manual smoke reference subordinate to the release checklist
- `docs/dependency-advisories.md` - reviewed RustSec findings that remain in resolved dependencies, including reachability and re-evaluation conditions
- `docs/tuning-native-probe.md` - actual module apply/restore evidence, recovery checks and remaining validation limits
- `docs/github-metadata.md` - manual GitHub UI metadata plan
- `CPU_SCHEME_INSTRUCTION` - format contract for `cpu_presets.json`

## Runtime architecture

The main navigation is Affinity groups / System Tweaks / Logs on Windows; System Tweaks remains absent on Linux. The group list has no repeated heading; its New group button is at the top right, absent from the shared header. The top row contains App settings with a gear icon on the left and the application version on the right; version hover shows the CPU model and logical thread count. The application title appears only in the native window title bar. The footer contains only monitoring controls/status and the tracked-process hover list.

The main navigation fills the available header width with 28-logical-pixel buttons. Selected main tabs use a small theme-derived rounded gradient mesh, 120 ms selection transitions, 80 ms hover and immediate pressed feedback; normal buttons retain hit testing, selection semantics and keyboard focus. Existing egui animation state owns the transitions, zero style animation time disables motion, and settled decoration requests no repaint. Panel content changes immediately and stays static. The System Tweaks section switch is centered, capped at 300 logical pixels and uses smaller flat 20-pixel buttons, keeping the primary navigation visually dominant.

Explicit in-app Close actions share a 44×21 logical-pixel button with theme-specific dark-red tokens and standard focus/hover/pressed behavior. The CPU graph has one Close at the top right. Group cards show their name without ordinal badges; their unchanged logical Group ID is read-only in Edit. Group and application names own a full-width stable drag target even when the text is short; separate drag grips are absent. Focused group names retain Up/Down keyboard reordering. Load meters are 47×18.4 logical-pixel percentage blocks with a horizontal rounded gradient fill and a visible status-colored border. Percentages remain centered over the fill. There is no separate vertical bar; zero/unknown load retains the status border without invented fill. Stopped processes remain neutral gray and noninteractive, mismatched running processes amber and matched running processes green; group meters retain the group accent. Decoration is static and requests no repaint.

Layers:
- `shell` owns the top-level `eframe::App`, tray/window lifecycle, route enums, UI sessions, presenter dispatch, and repaint policy
- `features` own product behavior:
  - `rules` owns group and rule mutations plus logical `GroupId` / `RuleId` identity
  - `execution` owns launch, process-instance revalidation, runtime tracking, reconcile loops, package-owner claims, and typed monitor notifications
  - `shortcut` owns saved-rule desktop shortcut service, shortcut filename allocation, OS adapter seam, and user-safe shortcut creation errors
  - `preferences` owns theme and monitoring toggles
  - `cpu_load` owns read-only Windows CPU graphs, independently of affinity/priority enforcement. One worker reuses 1 Hz system CPU samples for group means and exact PID/creation-token process totals. While CPU monitoring is enabled, one owned real-time scheduler trace collects tracked exact instances and host process contributions, independently of modal visibility or tray hiding. Process start/rundown/end metadata supplies bounded executable basenames and independent lifetime identities, including processes which start and exit between polls; no command lines, SIDs or kernel addresses are retained. New instances join shared accounting without restarting it; PID/token changes retire only the affected slot. Disable and orderly shutdown stop the trace; an empty saved/tracked set still records host contributors. Trace errors/loss and stale samples are unavailable, not zero. The worker rechecks tracked-membership generation before publishing process results, merges runtime instances by logical rule and samples shared instances once. Compact horizontal gradient meters replace the group load row and process status dots; process meter color retains settings compliance even at zero/unknown load, and clicking its percentage or bar opens the graph. Disabled CPU monitoring hides all load meters and their spacing; the existing text status remains. Modal charts use monotonic geometry, binary-search original-sample hover and local HH:MM:SS labels. Tessellated line/parking meshes and clock labels are cached by data, selected CPU, size, theme and pixel scale; pointer movement reuses immutable meshes.
    The system scheduler stream correlates a process end with the QPC time at which its pinned native lifetime was positively alive. Ends older than that observation cannot retire a reused PID. An admitted exact instance retains its final scheduler interval after confirmed exit and becomes known zero only in wholly subsequent intervals, including CPUs never scheduled by that dead instance. Target updates retain its ended accounting slot until ownership removes it, and new TIDs cannot charge that retired slot. Native checks stay outside the callback lock; trace loss, uncertain lifetime, startup and undelivered boundaries remain unavailable. The existing runtime memory estimate includes slot metadata. A 2026-10-16 elevated read-only browser/controlled-child probe reached complete samples for38selected instances on20logicalCPUs, retained the final exited-child work and then measured zero; this is a host probe, not universal gap-free coverage.
    The current compact group meter uses the complement of the documented PDH Idle Time counter. Modal system graphs instead use non-idle scheduler residency from the same original interval as their contributor rows. Interrupt accounting and whole-machine process percentages remain different metrics; unattributed scheduler work is explicit partial attribution. The same worker reuses read-only CPU Set topology to sample Parked flags once per second, mapping by processor group/logical index; missing or conflicting flags remain unknown. Original cut parking observations stay with their interval even when its load completes later. Load gaps remain gaps, never proof of parking. Disabling stops these reads too; no parking policy is changed.
    Event-time history uses original QPC/wall-clock cuts registered by the worker. Before applying a later delivered event, the callback splits scheduler residency exactly at each queued cut, closes the old target membership and then admits the next exact PID/token set. Sixteen preallocated cut/frame buffers bound delayed numeric storage; they are intervals, not a guaranteed number of seconds. The first cut establishes a baseline. Original rule/name/lifetime ownership snapshots stay attached to pending rows. Delayed completion fills that exact retained provisional row and its sparse contributors under one history writer; eviction, real loss, unknown schemas, out-of-order timestamps, pool overflow and restart never redirect work into a newer row. Lost rows cannot later become complete. Native lifetime checks stay outside the callback lock. Live tiles/table use the latest resolved original interval and show its clock while delivery is pending; a latest lost interval shows unavailable rather than older busy values. Hover/pin selects the exact original pending/completed/lost row. Curve, overview, tiles and contributors share one nonblocking read revision, and late completion invalidates cached geometry. The fixed native interval reservation enters the existing 256 MiB admission budget and settings estimate; Off releases it with the owned trace. No raw event log, second collector or per-CPU trace is added.
    Chart hover gives time, CPU, interval load and sampled parking on separate unwrapped rows, positioned within the viewport at either endpoint. Amber parking ticks sit below the curve at observed samples; adjacent flags never imply continuous parking. Interval load remains visible even when the CPU is parked at the poll. Both graph batches remain cached across pointer movement.
    CPU modals default to the current group's logical CPUs; All CPUs is transient and resets on opening. Process graphs retain a visible outside-current-mask activity count with a Show action even while filtered, and active outside tiles use red borders/text. Unknown data is not a clear result; earlier masks are not recorded. Each tile has a cached mini graph from at most 64 min/max buckets, retaining peaks and missing intervals. Clicking any tile selects its original-sample detailed graph below. Hovering the detailed graph reads one exact all-CPU historical row for tile percentages and sampled parking, marks the grid History with its local timestamp and draws synchronized mini-graph cursors; leaving restores Live without interrupting collection. Stable pointer movement reuses plot geometry and takes one UI pass; a changed layout can request one bounded relayout. Missing/evicted/contended historical data stays unavailable. All these reads use existing RAM history, with no extra native poll or per-CPU trace.
    CPU modals use 80% of usable window height with roughly ten-percent top/bottom margins, retaining compact width and a larger stable contributor viewport. Group charts have a transient Tracked processes only checkbox, defaulting to all host processes on each opening. It switches live CPU tiles, cached mini/detail history and contributor rows together. Tracked means the unique PID/creation-token union across all saved groups at the original sample; application graphs keep their own exact rule scope. The existing worker aggregates the already deduplicated tracked instances from the same scheduler sample into one additional preallocated per-CPU history ring. Historical contributor unions deduplicate exact lifetimes, retain original rule membership and distinguish reused PIDs. Missing aggregate data remains unknown and known sparse rows remain partial; scheduler failures are visible in tracked scope. The extra numeric ring enters settings estimates and the existing 256 MiB admission budget. Filter changes invalidate scope caches without creating a collector or native read.
    Below the selected CPU's detailed graph, a compact Process/PID/CPU table sorts contributions by descending residency on that logical CPU. Live uses the latest completed graph sample; hover uses that exact historical row with an explicit History clock. Group tables include observed host processes, not just saved apps; app tables retain the exact rule membership and creation-token instance for that sample. A preallocated sparse numeric ring shares bounded lifetime/name metadata (8192 historical identities) instead of dense PID/CPU/time matrices. Exceptional pool pressure evicts complete old contributor frames; missing/partial attribution is explicit and is never replaced with a current PID roster. Numeric history plus the contributor reservation stays within the existing 256 MiB admission budget. Modal host load and contributor rows share the original scheduler interval; unknown attribution remains partial and is never forced to match the compact current PDH meter. RAM-only cursor reads reuse cached rows; a fixed-height internally scrollable table keeps the plot stationary when historical process counts change. Settings include the contributor reservation and retained metadata in the estimated extra memory.
    A missing tracked sibling keeps the complete app/tracked-union total unknown, while a separately accumulated positive measured subtotal remains visible. The same worker fills bounded preallocated current subtotal vectors; historical quality uses the existing validity byte (unknown/complete/partial), adding no dense history ring. Exact-time tiles and detail percentages mark partial values with *, the selected header identifies Partial, and cached detailed/mini curves use dashes with hover/legend explanations. Complete values are solid, isolated valid samples are dots, and intervals with no positive known evidence or no data remain gaps. Quality survives retention resize and resets with overwritten rows, membership revalidation and Off; host busy-time graphs are unchanged. Settings include the small current subtotal allowance; no new trace or native read is added.
    CPU selection uses a contrasting two-pixel outline, selected fill, a painted check and an accent rail without changing tile geometry. CPU names use one shared themed rounded badge with a subtle outline in tiles and the selected detail header. Tile badges occupy 70% of the tile inner width and sit centered at the top; the percentage aligns right in the existing bottom parking row, leaving the name/check distinct. Parking keeps its own text color; activity outside the mask keeps its red warning alongside selection. The selected state is exposed through the tile's accessible button metadata and keyboard focus ring. The large graph and process table share one outlined card with a short matching header accent, equal eight-pixel insets and a selected-CPU badge; the load column and zero/unavailable messages name that CPU. Process names align left and numeric columns right in fixed cells. Static decoration starts no timer, native read or additional sampling.
    A left click in the detailed plot pins its exact original sample, keeping the cursor, all CPU tiles and process contributions at that time while the pointer leaves and the list scrolls. The top mode shows Pinned with its clock; pinned inspection suppresses the pointer tooltip and continues recording. The next left click anywhere in the application unpins once per physical event, preserving the clicked control's ordinary action and resuming hover or Live. Wheel and drag events do not unpin. Expired samples remain unavailable without nearest/current substitution; closing, reopening or disabling resets the transient pin. Selection state adds only bounded scalar UI data and no new collector or history allocation.
    Application CPU tiles additionally show a three-pixel green rail, inset three pixels from the right and twenty percent of tile height from top/bottom, when that exact rule has positive measured work anywhere in the currently retained window. Live/History/Pinned inspection does not change this window-wide marker. The cached raw-history overview pass computes one reusable boolean per CPU, retaining valid positive evidence even in an otherwise missing/gapped mini-graph bucket. New activity adds the rail; expiry, source removal or shorter retention removes it. This is observed CPU work, not process existence, current affinity or proof of complete observation. Group graphs do not show application rails. Selection checks/mini graphs reserve room, and theme success tokens provide the green; no extra native read, per-frame history scan or telemetry ring is added.
    App CPU modals have a bottom Graphs / Processes segmented switch. Processes replaces graph content with a virtualized current exact-owner PID list including idle and unavailable tracked instances, whole-machine CPU time and measured active logical CPUs; complete idle, partial and unavailable mapping remain distinct. The same 1 Hz worker retains each existing GetProcessTimes result before app aggregation and copies positive per-PID scheduler values, sharing immutable current rows with UI through Arc. Names are cached by exact PID/token after verified path/token inspection on the worker, independently of trace admission. Membership revalidation invalidates pending row values; source additions/removals and Off follow existing lifecycle. Table mode performs no native read or full-history copy; graph inspection/pinning is separate. The memory estimate includes current row storage without adding a historical per-PID matrix or collector. Footer and CPU telemetry derive ownership from RunningApps but have independent refresh/validation and display filters, so a tracked PID roster is not a selected-CPU contributor list.
    Unknown aggregate load still remains unknown, but exact known contributor rows are no longer hidden solely because a sibling instance is unavailable. Same-source, same-CPU, same-time sparse rows appear explicitly partial; an unknown interval with no measured contributor rows remains unavailable rather than idle. Current per-PID CPU time can remain readable while scheduler mapping is unavailable; its whole-host percentage and per-CPU residency have different normalization and interval boundaries. Every unique tracked PID is sampled even if an earlier sibling is unavailable; an unknown full total never short-circuits current per-PID collection. Scheduler collector errors are separate from tracked-membership warnings and appear in both host and tracked graph scopes.
  - `topology` owns CPU model/thread detection helpers
  - `diagnostics` owns startup logging, typed diagnostic event shape, bounded local crash-report formatting/capture, safe report discovery/retention, report-list state, and the Windows-only read-only Event Log lookup
  - `system_tweaks` owns schema-v11 profiles/associations, one module registry, an owned-field controller, compact disk recovery, one application worker and System Tweaks UI. Concrete managed settings and read-only cards live in `system_tweaks/tweaks/`. Native admission requires verified saved configuration, the recovery lease and resolved recovery evidence.
- `adapters` isolate storage loading, OS helper calls, and installed-app discovery
- `models` hold persisted schema plus domain and runtime-adjacent value types
- `runtime` is now a thin composition-root facade around `AppState`

Current runtime split:
- `shell::App` owns shell-only lifecycle state:
  - `tray_rx`
  - Windows local shortcut-forwarding server and primary guard
  - Windows tray icon guard
  - Windows `HWND`
  - Windows root `Arc<winit::window::Window>` for the renderer's retained taskbar policy
  - terminal tray-quit latch
  - hidden-window flag
  - Windows shutdown visibility: accepted close/quit requests synchronously hide the
    retained Winit root before draining background work. The queued visibility command
    keeps viewport state consistent while hidden logic polls shutdown. `App::on_exit`
    also hides before renderer teardown, including native-loop exit paths. Driver
    cleanup must not leave a visible empty window without a pumping message loop;
    CPU collector confirmation and tuning restoration retain their existing order.
- `runtime::AppState` is the composition root over:
  - `persistent_state`
  - `rules`
  - `ui`
  - `runtime`
  - Windows-only `cpu_load` sampling runtime and transient modal selection
  - `log_manager`
  - `crash_reports`
  - Windows-only `windows_event_log`
  - `tuning` request/snapshot runtime and typed module editor registry
- `shell::UiSession` owns transient UI-only state:
  - App settings parent-menu lifetime, independent of its nested history selector
  - the footer's retained tracked-process presentation rows, one-second refresh admission and unavailable-data status; ordinary GUI passes borrow the cached rows
  - active route
  - group form session
  - rule editor session
  - rule editor shortcut creation result/status
  - dropped files
  - installed app picker session and cached catalog
  - crash-report delete confirmations and the last report action message
  - Windows Event Log preference action error state
  - App settings menu startup action error state
  - System Tweaks section selection, help/undo confirmation and qualified saved-EXE target selection; existing saved profiles/associations remain compatible restoration data
- `features::rules::RulesContext` owns logical `GroupId` / `RuleId` allocation, index projection, and persisted `rule_identities`
- `features::execution::RuntimeRegistry` owns runtime process tracking:
  - `running_apps`
  - runtime-only installed-package metadata cache for Windows installed targets
  - package-owner claims for shared package-local helper processes
  - cached app statuses; a contention-time settings mismatch remains visible until a manual or monitor-confirmed successful settings application clears it
  - `monitor_rx`
- `RunningApps` commits a checked membership generation under the same writer as runtime-key, logical-owner and exact PID/token membership changes. No-op and settings-only edits leave it unchanged; overflow or interrupted membership mutation remains explicit uncertainty
- runtime process identity stays keyed by opaque `AppRuntimeKey`, but tracked app ownership now also stores logical `GroupId` / `RuleId`
- shell presenters are owned under `shell::presenters`; their source files still live under `src/app/views/` via path-based module ownership
- workers emit typed `shell::events::ShellEvent` messages and do not hold `egui::Context`
- Windows crash-report discovery uses an initial/on-demand single-flight standard thread plus at most one coalesced follow-up refresh; egui rendering reads only the last completed snapshot and synchronizes its newest validated report into the retained Logs context; the Linux beta does not start this worker or expose the Crash reports route
- Windows Event Log diagnostics uses a separate single-flight worker. It is never started by construction, focus gain, tray restore, or periodic refresh: after the first rendered UI frame it can run one bounded lookup when the persisted enabled-by-default preference allows it, with at most one delayed retry after a successful empty result.
- App settings offers a persisted `start_minimized` preference. The Windows entrypoint loads the canonical storage once before creating the root viewport, requests initially hidden/inactive/no-taskbar when enabled and passes that same storage into `AppState`. The shell starts hidden only with a usable tray; tray failure or blocked cold shortcut forwarding shows the error window. Explicit viewport visibility replaces offscreen positioning; restoring preserves geometry and ignores stale minimized input until Windows acknowledges restoration. Each taskbar transition updates both Winit's retained `skip_taskbar` policy and the existing native styles, including visible fallback, so `TaskbarCreated` cannot reapply an old hidden policy. A one-line local eframe 0.36.2 patch prevents its first-paint automatic show from overriding initially hidden roots. The preference defaults to false, remains readable in schema v10/v11 and affects only the next launch. No Windows login registration or `--start-in-tray` flag is added. See `docs/hidden-startup.md` for native evidence and remaining release smoke.
- App settings keeps monitoring, history and startup explanations in bounded hover tooltips. One `Extra memory: ~N MiB` row estimates monitoring's additional cost using calculated history, the configured maximum trace buffers and runtime/tracking allowances; its breakdown is on hover. This estimate follows retention, saved apps and observed tracking changes and is not measured whole-app memory. Preference and collector errors remain visible in the menu. Its lifetime remains independent of the nested history selector.
- Tray hiding also normalizes only the calling GUI thread's exact layered/nonactivating `Winit Thread Event Target` popup to a restored zero-sized state. The helper stays visible for Winit's paint-event delivery; other windows are not hidden or restored. Native regression coverage reproduces a leftover minimized helper rectangle; the Task Scheduler login scenario remains a manual smoke check.
- Hovering the footer monitor status opens a bounded tracked-process table; there is no separate list button. Process, PID and Group use fixed columns with a retained header, alternating compact rows and virtualized vertical scrolling. The scrolling viewport uses matching window-bounded minimum/maximum heights so a previously short or empty popup cannot constrain a later long list; auto-shrink keeps short lists compact. Hovering different status regions and reopening preserve the same scroll identity/position. Long cells clip within their column and support horizontal drag/Shift-wheel plus full-text hover, without intercepting ordinary vertical wheel scrolling or widening the popup. The list stays open while hovered, with a short gap-crossing grace period. Native refresh attempts occur at most once per second while open; GUI passes borrow retained rows, and unchanged completed rows keep their allocation. Unavailable runtime reads retain the previous presentation snapshot and scroll position with an updating label; an initially unavailable read shows a waiting message. A completed empty snapshot clears the rows. Closing the popup starts no polling. Each completed refresh includes only saved rules matching the runtime instance's logical group/rule owner, so duplicate executable rules do not duplicate or mislabel rows. It inspects only token-bearing runtime instances and verifies the current image path and creation token together before publishing the executable filename, PID, and group. Failed inspection or a reused PID is omitted; the popup never enumerates all system processes.

Windows runtime flow:
1. Entry point parses startup arguments into a narrow startup intent; normal GUI startup remains the default, while `--run-rule <group-id> <rule-id>` is accepted as a saved-rule startup intent.
2. Windows entry point prepares the local shortcut-forwarding endpoint before creating `tokio` or `eframe`:
   - normal GUI startup may claim the primary guard before GUI startup, but it never forwards, exits, blocks on an IPC lock, or becomes a global single-instance launch
   - `RunRule` startup first tries to claim the primary guard; if another primary owns it, the process forwards a typed `RunRule` command over the local IPC pipe and exits with the typed forwarding result code
   - if `RunRule` claims the primary guard, it cold-starts the GUI, starts the forwarding server after the shell owns the command receiver and GUI wake path, then skips normal autorun and dispatches only the requested saved rule
   - if the primary guard exists but the pipe is not ready, forwarding retries briefly and then exits non-zero instead of becoming a second primary
3. After startup forwarding selects `RunGui`, the entrypoint resolves the active crash-report directory, installs the main-thread panic hook, and prepares the startup phase before creating `tokio` or `eframe`; forwarding-only processes do not install the hook, and crash-report discovery or retention never runs synchronously on this startup path.
4. Startup self-scheduling is prepared before worker creation as described below. A main-thread panic is written best-effort without changing Rust panic semantics; background-thread/task panics delegate to the previous hook without creating a crash report.
5. Before Tokio/eframe/background-worker creation, the GUI process independently applies `BelowNormal` priority and a one-bit hard affinity mask for the highest available logical CPU in its inherited process/system mask. Native selection uses mask bits, including sparse restrictions, not physical-core count;4threads selects CPU3 and16selects CPU15 when all are allowed. Current startup affinity supports one Windows processor group and a representable native-width mask; multi-group/unrepresentable cases remain unchanged and are logged rather than wrapping or selecting a different global CPU. One original mask is retained for neutral EXE child launches, and the new self mask is read back. Both startup outcomes enter Logs; a failure in either does not prevent the other or GUI startup. Tokio is then created, and existing CPU-topology detection still reports the host CPU count. No periodic self-affinity enforcement or global policy is added.
6. The Windows entrypoint loads storage once, derives the root viewport's initial visibility/activation/taskbar request, marks the crash-report phase as UI-running and enters `eframe::run_native`.
7. The Windows entrypoint creates `App` without dispatching startup intent, passing the same storage to `AppState`. It seeds in-memory logical identities, writes startup diagnostics, starts execution monitors, captures `HWND`, initializes command-only tray integration, and starts the bounded crash-report index refresh. A hidden request becomes visible if tray initialization fails.
8. The Windows entrypoint installs the prepared shortcut-forwarding runtime with the GUI wake callback before dispatching startup intent.
   - normal GUI startup then runs autorun once
   - `RunRule` startup then skips normal autorun and dispatches only the requested saved rule
   - if a `RunRule` cold start claimed the primary guard but cannot start the forwarding server, the requested saved rule is blocked and logged instead of launching without an owned forwarding endpoint
9. If `run_native` returns `Err`, the entrypoint synchronously writes a typed native-loop report before preserving exit code `1`, then marks the report phase as closing.
10. `App::logic` handles tray events, monitor notifications, local forwarded shortcut commands, initial/on-demand crash-report refresh polling, the gated Windows Event Log worker, focus-gain refresh, hidden-window flow, file drops, and theme application; tray callbacks only enqueue typed restore/quit commands and request repaint, while `App::logic` owns all `HWND` restore/focus/taskbar work and requests an orderly root-viewport close for Quit. Once Quit is latched, no further shell work or rendering occurs before eframe shutdown. `App::ui` renders the active view; the Event Log worker may be armed only after that first rendered frame.

Linux entrypoint now reaches the shared `shell::App` shell, startup logging, autorun, and monitor wiring, but it still must not be described as having tray, taskbar, or focus parity with Windows runtime behavior.

Tuning status:
- Interactive user admission accepts the exact caller/Explorer identity or the caller's OS-queried UAC-linked token with the same SID/session and exact Explorer logon. User targets retain the desktop logon across elevation; other accounts/sessions and impersonation remain unavailable. Process token/lifetime identities retain their exact original contract.
- Persistent registry preferences, GPU preferences and power settings use a stable account SID resource identity. Each native operation still admits and pins the fully verified current caller/Explorer logon; another SID is denied. Module-owned target codecs normalize legacy logon-bearing recovery keys without changing baselines, values or owners; canonical collisions block recovery and retain the original evidence. Session-only mouse/accessibility/display targets keep their exact logon and are not transferred to another session.
- concrete registration supplies thirteen managed adapters (including five compact Windows preference/display adapters and the retained optional process timer-policy experiment) and ten read-only adapters without native work during construction
- registry registration supplies each module editor; the UI/controller do not match feature IDs or decode arbitrary native payloads
- the controller owns exact target/field first baselines, confirmed/attempted values and owners; it shares equal requests, rejects conflicts before writes and restores only owned fields at matching app-written state. New unowned fields already at the desired value acquire no baseline; existing equal owners still share the first original. Restore handles fields independently so drift or Unknown in one field retains its evidence without blocking safe siblings; a recovery publication fault stops subsequent writes.
- the single application worker saves state candidates off UI, reconciles existing bounded execution/foreground observations and publishes immutable snapshots; saved state/logical IDs adopt together only after successful persistence. Classic imports log success only after adoption, retain rejected input and carry accepted file input into completion for restoration or explicit failure reporting without overwriting newer input. Already accepted classic saves finish before shutdown restoration; autonomous display expiry reserves the existing admission latch and publishes its updated revision before reopening admission.
- compact recovery publishes original/current/desired/attempted field tokens and deduplicated owners before writes; one user-local writer lease, normalized selected-state source and exact saved activation facts control startup reconciliation. Invalid/ambiguous records or unavailable saved/native evidence remain blocked
- all eight managed adapters passed actual Windows apply/read/disk-reload/disable/original verification on 2026-10-08. Native admission opens only after verified saved state and recovery initialization; in-memory fixtures do not prove native readiness
- On 2026-10-10 all five new stored-preference/display adapters and the new AC-only power preset passed real Windows apply/read/disk-reload/disable/original verification. Display used 3840×2160, 32-bit color and 120↔144 nominal Hz, returning to the original144Hz; a bounded exact-mode settlement check handles delayed readback without accepting foreign modes. These results prove stored values and restoration, not game FPS, latency, HDR/VRR behavior or actual GPU renderer selection. Compact GUI validation was interrupted by the user's physical Escape; remaining live UI smoke is open.
- Hidden legacy read-only audio/PnP/USB inventory adapters are retained for compatibility but are not polled by the compact catalog; PC checks run only on explicit request. System ends its For games list with a manual Memory integrity guide: its explicit action opens the fixed Windows Security Core isolation URI through the existing worker. It does not read, change or restore HVCI, create ownership/baselines, or focus/highlight the native switch; manual changes are restored in Windows.
- schema v11 retains optional bounded tuning configuration; the compact version3 envelope stores profiles, rule associations, saved manual targets and monotonic profile allocation. Absence is Off. Unknown extensions retain exact raw input read-only; malformed/oversized input and old nonempty activation/withdrawal/transition evidence remain blocked
- process identity uses PID, creation token and image, with an actual retained native lifetime guard; only positive completion of the same instance retires its process owner, while observation errors remain Unknown
- Manual owners retain their durable baseline/settings across application closure until disabled or their temporary native session is positively proved ended. Mouse/accessibility/display adapters use bounded read-only WTS queries under the existing worker, with the current same-account Explorer/UAC context verified before and after. New session targets additionally retain the terminal SessionId and WTS LogonTime; native reads/writes verify that exact instance. A surviving LSA authentication record is not proof of a surviving interactive desktop: legacy records supply their verified account/terminal binding, while a direct missing-session result requires a consistent successful WTS inventory. A recorded epoch also detects reuse of the same terminal number; a legacy record with an existing terminal remains conservative. Lock/disconnect is not expiry. Denial, unsupported/incomplete data, inconsistent inventory or a changed context retains evidence; session verification errors have their own status rather than external-setting drift. Saved-source-admitted reconciliation retires only that expired module target, including shared owners, without reading/writing its vanished settings; failed durable publication restores in-memory evidence and closes admission. Persistent account targets are unchanged. Quit restores Session owners only with current native/legacy/saved-source admission; exact positively expired process fields retire without a dead-PID read/write. Shutdown uncertainty retains evidence and reports RecoveryRequired
- CPU Sets/priority integration uses per-runtime field-selection handoff with existing launch/monitor writers; no second lifecycle monitor is added
- System Tweaks uses one permanently visible horizontal segmented switch with three sections: System, GPU / network, and Check PC. Full section descriptions are available on hover. System opens by default and contains five game settings and two comfort settings with direct Apply/Undo, visible scope/current value and explanations in help; the manual Memory integrity guide follows the game settings before For comfort. Only the selected content scrolls; section selection is transient and starts no native operation. GPU preference targets one qualified saved EXE; hardware guides remain manual and Check PC requires an explicit read-only request. Global errors, Undo and display confirmation remain reachable across sections. There are no visible profiles, triggers or duplicated process affinity/priority controls. Each manual tweak reuses one hidden existing-schema profile/target intent. Old profiles, associations and recovery data remain retained internally; only old module settings with actual owned fields appear in a conditional block at the bottom of System, without profile names or saved activation flags. Each old setting has its own Undo, which disables only that saved module and uses existing conditional restoration. Unresolved ownership remains visible for retry even after withdrawal; no empty restoration block or separate Restore section is shown. Already-suitable values create no owner or Undo baseline. Cards distinguish exact stored preferences from effective/game results, restart requirements, drift, failure and restoration.
- Display refresh changes only a tested compatible nominal Hz at the current geometry/color/scan mode and requires its exact original mode to be in the tested inventory before acquisition. It is a temporary user-session change with a 15-second confirmation; pending intent is durably distinct, and expiry, closing or startup attempts exact restoration. Failed withdrawal retains evidence and reports recovery required. Confirmed settings are not written as persistent boot display modes. An unowned display card rediscovers the current primary screen; active or unresolved ownership retains its exact original target.
- New GPU preference activation qualifies an existing saved EXE. Owned registry reads and conditional restoration retain the exact user/path even after EXE removal; UNC qualification preserves the network root. This does not bypass user/session or expected-value checks.
- The Windows viewport has a fixed width of 400 logical pixels and permits vertical resizing from 600 to 1000; the tuning page is bounded to 400. Tweak cards, including manual guides, fill the available width and reserve a fixed right action column; Undo replaces Apply in the same slot. The remaining application layout and classic saved-EXE workflow remain shared.
- AC power now exposes one direct Apply/Undo for the Windows AC performance preference. CPU frequency sliders, high-clock and parking controls are retired; configured requests for those options are rejected. New applies contain no CPU policy or DC fields, and observation no longer queries CPU topology. Legacy saved CPU options, exact base/Performance-overlay bindings, wider values and nullable originals remain intact for recovery and Undo; the card identifies previous CPU settings. Existing owned changes must be undone before applying the simpler preset. Verification describes the stored Windows power preference, without asserting physical clocks or FPS.
- CPU AC overrides retain absent versus explicit DWORD originals. Scoped caller-token SeRestorePrivilege is checked before creating an override and restored immediately; exact absence Undo reopens a retained setting object with RegCreateKeyExW and an empty subkey, deletes only ACSettingIndex, and never changes ACLs or deletes setting containers. A module-owned confirm_applied hook repeats guarded runtime refresh for matching recovered bytes or an already-restored baseline before retiring evidence. Other adapters use its no-op default. Refresh never selects a different captured plan. Real Windows 11 limit 4500 MHz/high-clock-off and high-clock-on round trips passed on 2026-10-15, including disk reload, exact absence Undo, unchanged active plan/DC preference and parking; this does not prove measured clocks or FPS on other hardware.
- old canonical continuity/configuration-receipt/handle-transfer laboratory source was removed with recoverable local snapshots; existing ProgramData journal/staging/previous evidence is detected read-only and never discarded or overwritten
- native handles, typed user/process reads and conditional writes, and read-only display/audio/PnP inventories live in `libs/os_api`; application code owns configuration and lifecycle
- tests and contribution rules are in `src/app/features/system_tweaks/README.md`; product scope is `docs/modular-gaming-features-plan.md`
- System Tweaks lives entirely in `src/app/features/system_tweaks/`; each managed tweak owns a folder under `tweaks/<name>/mod.rs`. `platforms/windows.rs` composes the Windows catalog; `platforms/linux.rs` is an inactive extension point. Each adapter declares its Windows 10/11 minimum build. Cached read-only real Windows build detection gates observation and activation; unsupported cards remain grey with a reason. Recovery/Undo is retained independently of activation eligibility. Serialized `tuning` data, stable feature IDs and recovery targets remain compatible. Windows 10 compatibility is declarative and unit-tested; native round-trip evidence remains from Windows 11.
- Operation feedback belongs to the current request: a new request clears the previous transient error/restore result while unresolved controller evidence remains visible. Autonomous retries clear their own prior errors; confirmed withdrawal forgets rejected attempts only without field/record/pending evidence. Restoration preserves the strongest restart requirement and distinguishes partial results. Disabled platforms never reserve the classic affinity/priority fields from retained tuning associations.

## Concurrency model
- GUI runs on the main thread
- System Tweaks session observation/reconciliation runs only for enabled non-manual rule associations or retained Session ownership. The worker caches that demand across idle queue checks and reconciles at most once per second or immediately after an accepted request; manual-only/undone configuration does not start process observations or repeated saved-state verification. Native mutation, manual inspection, display expiry and shutdown retain their independent verification paths.
- Positively expired compact session ownership also withdraws its saved manual intent. An unowned non-EXE card discovers the current native target, including after interruption between recovery and intent publication. At the 64-profile admission limit, new compact targets may reclaim only disabled hidden profiles without associations, controller records, pending/error evidence or display confirmation; legacy profiles and monotonic IDs remain intact. Unresolved Undo is never capacity-reclaimed.
- Classic save methods return queue acceptance in all builds. Tests that need completion explicitly call a test helper; gated tests distinguish pending, successful and failed persistence. The retired transient profile editor is absent; saved legacy configurations and conditional Undo remain supported.
- background tasks use `tokio`
- the tuning application runtime uses one dedicated standard thread and bounded request queue for saved candidates, owned-field operations and execution observations. It publishes immutable snapshots; shell polling adopts them while hidden and during bounded shutdown. Native reads never hold GUI-facing snapshot/admission locks.
- Full System Tweaks card/capability refresh runs only while its page is visible and uncovered. The worker continues owned-field inspection, execution/session reconciliation, display-confirmation expiry and shutdown recovery while hidden. Reopening requests a fresh card snapshot on the same worker. Compact cards use a combined observation/readiness seam; display refresh reuses one tested mode inventory per card refresh. Apply/Undo still perform their independent native validation.
- CPU graph modals suppress rendering of the underlying active page, keeping the header/footer and ordinary eframe scroll distance and input. During modal wheel smoothing or dragging, a 90 Hz frame budget admits paints on the existing GUI thread with a bounded wait of at most one 11.12 ms interval. Buttons, keys, text and viewport close bypass it immediately. No input is discarded, no new thread or idle timer is added, and actual cadence depends on native presentation. Uncovered pages and non-scrolling idle/hover retain reactive rendering. Each retained original CPU interval additionally stores a bounded one-byte gap category (baseline, numeric contribution capacity, stream/schema/order failure, missing frame or other known failure). Native frame causes propagate to MCP quality and opt-in render logs; resize/overwrite/Off preserve the original-time lifecycle. A category never turns an unknown interval into zero or asserts a more precise cause than was observed.
- Windows shared-font preparation feeds egui's own caches in at most eight-character portions once per physical UI frame. The fixed font/character set restarts at a changed pixel scale; temporary eight-millisecond repaint requests end after preparation completes. It paints no hidden screen, adds no worker or second text cache, and does not pre-render arbitrary Unicode process names. CPU tiles reuse one measured size qualified by width, scale, font height and spacing; fully clipped tiles reserve that geometry without laying out their labels. Exact current PID display strings follow existing one-second snapshots, invalidation and memory accounting; contributor PID/load strings follow the existing exact-time view cache. Table virtualization, cursor selection and telemetry frequency are unchanged.
- The fixed native numeric interval reservation allows 128 positive contribution cells per logical CPU (at least 256), retaining all 16 pending/frame buffers. This startup/churn headroom is reserved once on enable, included in settings estimates and the same 256 MiB admission budget. Exceeding it remains an explicitly categorized gap; buffers do not grow in the event callback.
- tray callbacks only enqueue typed commands and wake egui; `tray_rx` is owned and drained by `shell::App` on the GUI thread, which exclusively owns `HWND` operations and the terminal Quit transition
- Windows local shortcut-forwarding requests flow through a shell-owned named-pipe server thread into `shell::App`, with per-request reply channels; request enqueue wakes the `egui` context for prompt draining
- `AppForwardingRuntime` clears the GUI wake callback and requests server shutdown during drop. It joins promptly when the worker reaches a terminal I/O completion; otherwise the detached reaper retains the pipe listener, in-flight `OVERLAPPED` buffers, and primary guard until it can safely release them. A replacement process therefore cannot claim the endpoint while that worker still owns it.
- monitor notifications flow through typed `ShellEvent` messages in `monitor_rx` owned by `RuntimeRegistry`; runtime writer guards are released before emitting notifications
- `RuntimeRegistry` also carries an optional GUI wake callback for installed-catalog completion and post-launch tracking changes; callbacks run after publishing results and releasing runtime writers
- a contended UI status read retains its cached value and requests another repaint after 50 ms
- persisted state uses `Arc<RwLock<AppStateStorage>>`
- running-process tracking uses `Arc<TokioRwLock<RunningApps>>`
- installed-package runtime metadata cache and ownership state use in-memory `Arc<RwLock<...>>`
- Windows tray integration uses tray-icon and muda event handlers instead of a polling loop; their process-global callbacks are installed once for the GUI process lifetime, never capture or operate on `HWND`, and retain only the typed command sender plus repaint context
- crash-report capture uses immutable precomputed context, a thread-local recursion guard, and a non-blocking process-wide writer guard; the hook does not take app-state or GUI locks

Background loops:
- running-process rediscovery and retracking loop: narrow borrowed-state projections avoid cloning whole rules; an empty eligible configuration skips process enumeration while still reconciling stale tracking and package-owner claims
- affinity and priority verification and optional correction loop: copies only rule identity, name, affinity, and priority needed by the iteration
- bounded Windows crash-report discovery/retention worker with no overlapping replacement worker after timeout; completion is polled briefly after explicit refresh, while a timed-out worker does not force perpetual high-frequency repaint
- separate bounded Windows Event Log worker with no overlapping replacement after timeout; disabling diagnostics cancels the delayed retry and uses a generation token to discard an in-flight result

Hidden-window flow:
- forwarded shortcut commands are drained in `App::logic` before the hidden-window render skip
- when the window is hidden, `shell::App::logic` preserves the 250 ms `ctx.request_repaint_after(...)` fallback and `App::ui` skips rendering
- the hidden-window path no longer sleeps on the UI thread

## State and data contracts
State split:
- `AppState` is the runtime facade over persisted state, transient UI state, runtime registry, and logs
- `AppStateStorage` is the persisted JSON schema

Persisted state facts:
- if `state.json` already exists next to the current executable, that legacy sidecar path remains the active persisted state location for the whole run
- otherwise the default persisted state location is platform-correct:
  - Windows: `%LOCALAPPDATA%\CpuAffinityTool\state.json`
  - Linux: `${XDG_DATA_HOME:-$HOME/.local/share}/cpu-affinity-tool/state.json`
- there is no automatic migration or copy between the legacy sidecar path and the platform data path
- current persisted schema version: `11`
- Optional `cpu_load_monitoring_enabled` defaults to true, is retained in ordinary state saves and can stop all Windows CPU telemetry without changing `process_monitoring_enabled`. App settings explains monitoring overhead and stores `cpu_load_history_seconds`: 30, 300 or 1800 seconds, default 300. Histories are memory-only numeric rings, reserved for every saved rule when monitoring starts and bounded to the selected elapsed duration plus one endpoint. Timeline and parking storage are shared, and recording overwrites preallocated cells. Only the visible CPU series is copied into a reusable UI cache; published current-value snapshots do not clone history. Adding a rule reserves its own ring with no pre-addition samples; deleting it releases that ring, and moving a logical rule preserves its history. Existing histories survive process restarts. Retention changes preserve available points without restarting ETW or increasing the 1 Hz cadence; increasing retention cannot recover evicted points. Disable releases all history. Calculated history size plus estimated native buffers/runtime/tracking tables is shown in settings and follows saved-rule, retention and observed membership changes. History admission is capped at 256 MiB with checked, fallible reservation; an oversized request reports an error without changing affinity/priority protection. No CPU-load data or trace files are persisted. Linux retains the preferences but starts no collector. Real-time ETW sessions are stopped on normal closure; an abrupt process termination can leave an OS-owned session alive, so this is not a crash-cleanup guarantee.
- On 2026-10-15 a real elevated Windows 11 CPU-load probe passed: 20 logical-CPU PDH samples, exact-process aggregate and per-CPU scheduler residency during a five-second controlled load, wrong-token refusal, prompt own-session shutdown and an independent logger remaining active. The observed 28-byte CSwitch v5 event uses a source-confirmed bounded ID prefix when local TDH metadata is unavailable; unknown versions still require TDH. This proves native sampling and normal cleanup on that host, not measured game/FPS overhead or universal hardware compatibility.
- On 2026-10-15 the optimized continuous-history probe passed on the 20-logical-CPU Windows 11 host: ten saved-rule histories shared one exact controlled PID with one arithmetic/eight sleeping threads, 1800-second capacity reserved1.951MiB,25seconds recorded before opening. Graphs closed:0.0195% whole-machine CPU (0.3906% one-CPU equivalent),+8.012MiB private/+8.539MiB working set. Addition/deletion, sibling retention, disable ring release and worker stop passed; the native own/foreign-session probe also passed. Details and measurement limits are in docs/release-smoke-matrix.md; these are not universal FPS, GUI/GPU or allocation-free lifecycle claims.
- schema `v5` and older formats are dual-read and normalized in memory without eager rewrite on load
- schema `v6` and older path-target app rules receive an in-memory one-time compatibility backfill that adds the primary executable filename to `additional_processes` when no normalized equivalent already exists
- schema `v7` treats an empty `additional_processes` list as intentional user state and does not re-add the primary executable filename on load
- schema `v9` stores `windows_event_log_diagnostics_enabled`; the pre-release schema-v8 disclosure field is ignored, and v8 and older files are effective enabled without an eager rewrite
- schema `v10` stores each rule's `manage_descendants` policy. Version-aware loading materializes missing values in pre-v10 state as the previous behavior (`true`); missing values in v10 and later, schema-less rule inputs, and new rules use the safer `false` default.
- schema `v11` adds optional bounded tuning configuration. v10 and older inputs without it remain Off and are not rewritten on load. An unexpected tuning extension in an older state blocks an upgrade that could reinterpret it as activation. Unknown v11 top-level fields and tuning structural versions/fields preserve their exact raw values through unrelated saves
- state reads consume at most 16 MiB plus one detection byte. Oversized input remains on disk with a finite save refusal; bounded unsupported/future or malformed explicit version headers and duplicate top-level names retain raw evidence and a private save block. Tuning evidence is guarded before unrelated base-schema decoding can enter recovery. Clone and explicit schema preparation cannot clear that block. Serialization is bounded to the same 16 MiB before backup, directory creation, staging or publication. Ordinary syntactically invalid JSON below the bound retains the existing copy-before-default recovery path
- the upgrade from pre-`v6` data or `v6` data to the current schema happens only on an explicit save path
- before the first current-schema save after loading pre-`v6` state, persistence creates an additional `state.json.pre-v6`, `state.json.pre-v6-1`, and so on backup series
- loading `v6`, `v7`, `v8`, `v9`, or `v10` for upgrade to `v11` does not create a `pre-v6` backup
- after the first current-schema save, downgrade to an older binary that only understands earlier state is unsupported
- backup rotation uses `state.json.old`, `state.json.old1`, `state.json.old2`, and so on
- persistence loading is split into `state_path`, `storage_io`, `migrations`, and `schema_refresh`; saves stage and sync a same-directory temporary file before an atomic replacement on Windows or an atomic rename plus directory sync on Linux. Windows does not claim parent-directory durability across power loss; recovery and migration backups are copy-and-sync operations that preserve their source before publishing.

Key entities:
- `CoreGroup` - CPU core group plus assigned apps
- `AppToRun` - application launch configuration with `Path` or `Installed` launch targets
- `GroupId` / `RuleId` - logical persisted identities for groups and rules
- `AppRuntimeKey` - opaque runtime-only identity derived from `AppToRun` for tracking and monitor lookups
- `RunningApp` / `RunningApps` - tracked live processes
- `RulesContext` - logical identity catalog and index projection over persisted groups and rules
- `CpuSchema`, `CpuCluster`, `CoreInfo` - logical CPU layout description
- `LogManager` - in-memory runtime log and history
- `CrashReportManager` - runtime-only single-flight report index, last complete snapshot, and retention/action facade
- The nonshipping Windows `render-profiling` feature adds explicit environment-opt-in local JSONL diagnostics under `features/diagnostics/render_profile.rs`. Ordinary builds compile out the scope clocks and calls. The diagnostic build logs inclusive wall time and unscaled scheduled thread cycles for static UI/graph/collector/discovery stages, frame IDs/view/input counts and previous-frame eframe timing, plus one-second own-process CPU/RAM and original history interval transitions. A bounded256-record queue feeds one writer, with64MiB/time bounds, visible drop/truncation/completion counts and no renderer/callback disk I/O, forced repaint or second scheduler trace. F8 adds a user stutter marker. No process names, paths, command lines or saved settings enter the profiling log. The separate `scripts/Start-render-test.ps1` package uses an isolated sidecar, paused enforcement/no autorun/tuning and local-only ZIP output; normal release contract and Desktop build stay unchanged. These logs identify instrumented work and remaining backend/scheduling time, not GPU execution or OS-input-to-display latency.
- The Windows `debug-mcp` feature is additionally gated by `debug_assertions`; the build script rejects the release profile even if debug assertions were manually enabled, and the source rejects non-debug-assertion builds. Ordinary release builds exclude its module, GUI hooks and endpoint strings. Explicit session environment variables and an isolated sidecar marker enable one authenticated loopback RPC worker with bounded requests/deadlines; no listener starts without them. `features/diagnostics/debug_mcp*` owns transport and egui input/frame responses; `cpu_load/debug_api.rs` exposes existing graph state and original rendered table rows. Renderer screenshots are explicit and marked as timing perturbations. The stdlib-only `scripts/cpu_affinity_mcp.py` adapts that local session to stdio MCP, reading its key from a per-launch ticket, with no proxy or redirects. It controls graphs, tables, monitoring preferences, ordinary guarded System Tweaks widgets and orderly test-app closure, without direct OS-command, rule-launch, affinity or tuning RPCs. `scripts/Start-mcp-debug.ps1` creates isolated settings with enforcement, autorun and tuning disabled, and elevates only this test app for ETW. The separate Desktop package uses the optimized dev profile; idle RPC waits do not request GUI repaints. Codex connection configuration is local and is not repository runtime input.
- `WindowsEventLogManager` - runtime-only Windows-only single-flight read-only Event Log lookup state

Important contract facts:
- persisted `theme_index` values map to native egui preferences: `0` follows the system theme, `1` forces light, and `2` forces dark; shared widget styling is applied to both egui theme styles
- `additional_processes` in `AppToRun` is the persisted backing field for the user-visible Tracked Process Names list and participates in runtime process matching
- path-target app rules use their visible primary executable process name for exact process-name plus image-path verified tracking; other visible tracked process names are exact user-controlled fallback matches
- `AppToRun` path targets store both source path and resolved executable path
- `AppToRun` installed targets store Windows `AUMID` and do not expose user-editable args in the current contract
- runtime tracking identity keeps the existing stable encoded key contract, but it now flows through typed `AppRuntimeKey` instead of raw `String` keys across runtime core
- tracked process IDs carry a process-instance creation token. A PID is not retained merely because it is live: rediscovery rebuilds roots from current verified provenance and associates only current token-bearing processes; descendant expansion is governed by the persisted rule policy.
- logical group and rule ownership persist in `AppStateStorage.rule_identities` starting with schema `v6`
- `rule_identities` also persists next group/rule allocation counters so deleted logical IDs are not reused after save and reload
- older `rule_identities` data without allocation counters remains readable; missing counters are reconstructed from the highest existing logical IDs and persisted on the next explicit save
- when pre-`v6` state is loaded, `AppState` seeds in-memory logical identities immediately and persists them on the next explicit save
- the Windows `Find Installed` picker is a launch-safe Windows subset backed by `AppsFolder + Start Menu shortcuts + App Paths`, not a full OS inventory
- tracked Windows installed targets now use a runtime-only package metadata cache plus package-local PID enrichment while the target stays tracked
- package-local helper PID ownership for multiple installed targets in the same package follows `first active target wins`
- `AppStateStorage` may rebuild `cpu_schema` for the current machine through presets when the stored schema is generic or outdated for the detected CPU model
- `LogManager` keeps a bounded in-memory chronological history with three retention classes plus the retained local crash-report context:
  - `Regular` capped at 1000 entries
  - `Important` capped at 200 entries
  - `Sticky` retained outside normal rotation for startup and critical diagnostics
  - the local crash-report context is runtime-only and separate from chronological entries; the **Clear** action in Logs preserves it
- local Windows crash reports are separate from `AppStateStorage` and do not change schema `v10`:
  - directory: `<active-data-dir>/crash-reports/`
  - event kinds: main-thread panic and native UI-loop error
  - maximum complete report size: 256 KiB; maximum payload section: 8 KiB
  - only complete recognized UTF-8 reports with the format-v1 completion marker enter the visible index
  - newest 20 complete reports are retained; incomplete, corrupt, unrelated, nested, and reparse entries are never removed automatically
  - each panic-path writer refuses a new report after it observes 64 managed complete or partial entries; this is a non-transactional multi-process safety ceiling rather than a strict quota, so simultaneous GUI writers may exceed it slightly
  - no crash-report scan or deletion is added to the synchronous startup path; a successful normal background refresh prunes complete reports to 20, while incomplete or invalid entries require user review
  - after a completed background scan, Logs shows the newest validated report's type, timestamp, reason, and full-report path; the report file remains the complete support artifact
  - reports are never uploaded automatically and can contain local paths or system details
- Windows Event Log diagnostics is separate from crash reports and remains runtime-only, owned by `WindowsEventLogManager` rather than `LogManager`:
  - it reads only recent local `Application Error` Event ID 1000 records from the local Application log after the first rendered frame when the enabled-by-default preference remains on
  - it uses an exact executable-name plus fully-qualified-path match and accepts false negatives rather than filename-only matches
  - only record ID, UTC time, exception code, a sanitized faulting-module basename, and strictly validated optional module version, faulting offset, and process creation time may enter the retained Logs context; no raw XML, event payload, path, clipboard, state.json, crash report, dump, or automatic upload is used
  - an Logs record is unverified supplemental evidence, not proof of a previous launch or crash cause
  - Logs can disable the lookup persistently; disabling clears the visible record immediately, while a failed save is reported as session-only. The app never creates dumps or changes WER/registry settings.

CPU presets:
- `assets/cpu_presets.json` is a compile-time source file
- presets are embedded into the binary through `include_str!`
- changing `assets/cpu_presets.json` requires a rebuild
- `CPU_SCHEME_INSTRUCTION` defines the preset format and editing rules

Data source separation:
- `state.json` - runtime state
- `assets/cpu_presets.json` - compile-time embedded source
- `changelogs/*.txt` - manual release notes, not runtime input
- `crash-reports/*.txt` - local bounded support artifacts, not persisted application state and not automatic telemetry

## Platform boundary
`libs/os_api` is the main boundary between the app and the OS. It covers:
- tuning boundaries: exact current interactive-user/process identity, retained same-instance process lifetime guard, typed conditional gaming-setting writes and read-only legacy recovery detection; no continuity or receipt framework
- Windows local named-pipe and mutex transport for saved-rule shortcut forwarding
- process launch
- installed-app discovery and activation on Windows
- installed package metadata lookup on Windows
- opening the active data directory in the platform file manager
- opening crash-report directories and selecting report files through an Explorer shell process whose token is verified non-elevated and below high integrity; the pre-existing Logs data-folder action retains its direct Explorer launch contract
- bounded read-only lookup of matching local Windows Application Event Log records for enabled-by-default diagnostics
- process-instance token lookup used to prevent retained/reused PIDs from being managed as a different process instance
- Windows affinity/priority application labels the failed Win32 operation and, after an access-denied settings open, may read `ProcessProtectionLevelInfo` through a query-limited handle to explain a confirmed protected-process denial; it never changes protection or privileges
- resolving the current elevated token's per-user Desktop directory for Windows shortcut creation
- Windows shortcut creation for saved-rule launch shortcuts
- affinity read and set
- priority read and set
- process inspection and process-tree logic; Windows single-PID parent checks use a fresh Toolhelp snapshot and an early-exit scan without allocating whole-system maps, preserving process-instance validation
- process AppUserModelID lookup on Windows
- window focus and visibility helpers
- URI and shortcut resolution
- CPU model detection

Internal backend structure:
- Windows backend is split internally into focused modules under `libs/os_api/src/windows/`:
  - `common`
  - `ipc`
  - `scheduling`
  - `processes`
  - `shell`
  - `launch`
  - `window`
  - `cpu`
  - `event_log`
  - `tuning` - current token/process identity and retained process-lifetime observations
  - `gaming` - bounded typed Windows gaming-setting operations
  - `gaming_diagnostics` - read-only display/audio/PnP inventories and fixed Windows Settings actions
- crate-root public shape remains intentionally narrow: external callers still interact through `OS` plus small boundary value types such as `PriorityClass` and `ShortcutSpec`
- Linux backend remains a single-file minimal backend and is not forced into parity with the Windows internal layout

Windows release-path surface:
- Windows root file import uses Shell `WM_DROPFILES` rather than Winit's OLE target, allowing ordinary Explorer to send paths to the elevated GUI. Only that root's drop, copy-data and Shell global-data-transfer messages are admitted. Bounded input retains the original client release point through DPI conversion and pending saves; imports never launch the dropped target. Shell reports the final drop and the system acceptance cursor, without OLE hover previews. Registration/filter/procedure ownership is released on window teardown. Linux input is unchanged; see `docs/explorer-file-drops.md` for native acceptance evidence and limits.
- tray integration
- tray initialization falls back to a reachable normal window when the icon cannot be created; it never hides the window without a usable tray
- taskbar and focus behavior
- `.lnk` and `.url` parsing
- `.lnk` creation through `os_api::ShortcutSpec`
- current-token per-user Desktop resolution through the Windows known-folder API for saved-rule shortcut creation; credential-over-the-shoulder UAC can place shortcuts on the elevated account's Desktop instead of the unelevated shell user's Desktop
- token-verified Explorer-process shell execution for crash-report folder open and report selection; elevated/high-integrity or unknown brokers fail closed, and the app does not launch a default text editor for managed crash reports
- local named-pipe and primary-guard forwarding for saved-rule shortcut launches
- bounded named-pipe shutdown with an ownership-preserving worker reaper for incomplete overlapped I/O
- registry-based URI resolution
- `AppsFolder + Start Menu shortcuts + App Paths` installed app discovery and AUMID activation
- runtime-only package metadata lookup and package-local helper tracking for installed targets
- richer process inspection
- embedded release manifest and resources
- Windows release-path CI validation
- current published release artifact

Linux backend surface present in repo:
- `/proc`-based process inspection
- `.desktop` parsing
- `.desktop`-based installed-app catalog discovery for the picker
- query-matched `PATH` executable discovery for the Linux picker
- `xdg-mime` URI lookup
- affinity and priority via `nix` and `libc`

Linux gaps:
- no tray parity
- no focus parity
- no crash-report capture, index worker, Crash reports UI, or Explorer-broker parity
- no Windows Event Log diagnostics capture, worker, or Logs context
- no Windows-style installed-app activation, AUMID identity, or package metadata parity
- `os_api` is not symmetric between Windows and Linux
- no Linux stable release artifacts, installer packaging, AppImage, Flatpak, or parity with the Windows stable release contract

## Dependencies and tooling
Only list materially relevant dependencies by actual role.

Primary runtime and build dependencies:
- `eframe` / `egui` - desktop GUI; the root Cargo patch selects the checked-in eframe 0.36.2 copy with its single initial-visibility correction (`vendor/eframe/LOCAL-PATCH.md`), leaving other dependency versions and the event loop unchanged
- `tokio` - background runtime with `rt-multi-thread`, `time`, and `sync`; worker count remains the runtime default
- `windows` - Win32 bindings for shell integration, process/runtime operations, local IPC, security descriptors, and manifest/resource-adjacent Windows APIs
- `tray-icon` - Windows tray integration
- `rfd` - file dialogs
- `serde` / `serde_json` - persisted JSON schema
- `regex` - CPU preset matching and related helpers
- `once_cell` - lazy initialization
- `num_cpus` - logical thread-count detection
- `image` - explicitly selected ICO tray decoding with only the `ico` feature; the decoded RGBA buffer is moved into the tray icon
- `raw-window-handle` - direct Windows native-window handle access
- `winit` - the same locked GUI dependency, also explicitly selected on Windows for its existing taskbar policy API and test-thread event-loop extension; no new event loop or input layer is added
- `winres` - Windows resource embedding at build time
- `libs/os_api` - local platform abstraction crate

Both application binaries use the platform system allocator; the project does not install a custom global allocator.

Linux-only backend dependencies inside `libs/os_api`:
- `nix`
- `libc`
- `errno`
- `shlex`

Do not invent dependency purpose just because a crate appears in `Cargo.toml`.

## Build, verification, CI, and release

The root package requires Rust 1.95 or newer, matching the `eframe` / `egui` 0.36 desktop stack. CI and release workflows use the stable toolchain.

Local verification commands:
- `cargo test --manifest-path libs/os_api/Cargo.toml`
- `cargo test --features windows --bin cpu-affinity-tool`
- `cargo fmt --all -- --check`
- `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/build-windows-release.ps1`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-pdb-verifier.ps1 -ExePath target/release/cpu-affinity-tool.exe -PdbPath target/release/cpu_affinity_tool.pdb`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/assert-windows-release-manifest.ps1 -Path target/release/cpu-affinity-tool.exe`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-crash-reports.ps1`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-tuning-recovery.ps1`
- `cargo test --features linux --bin cpu-affinity-tool-linux`
- `cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings`
- `cargo build --release --features linux --bin cpu-affinity-tool-linux`

`cargo make`:
- local developer automation wrapper around tasks like `fmt`, `lint`, `build-release`, `check`, and `release`
- not the release source of truth
- CI and GitHub Release workflows do not rely on `cargo make` as the truth source

Current CI facts:
- runners:
  - `windows-latest` for the Windows release-path job
  - `ubuntu-24.04` for the Linux desktop beta job
- `.github/workflows/ci.yml` cancels superseded runs per branch or PR, restores Rust build cache, keeps the Windows release-path checks on `windows-latest`, runs feature-gated real-binary crash probes and isolated forced-process tuning recovery probes, reproduces the stable line-table release build, verifies the built EXE/PDB identity and manifest resource, and verifies the Linux beta binary on `ubuntu-24.04`
- tests are part of the committed CI contract for `ci.yml`; both jobs require committed lockfiles to remain unchanged after compilation/tests, and the Windows release build reports EXE/PDB byte sizes. The Windows job also runs the MCP launcher's filesystem isolation self-test without launching an app or requesting elevation.
- the Windows CI job validates the feature-gated Windows binary path explicitly with `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --features windows --bin cpu-affinity-tool`, `scripts/build-windows-release.ps1`, `scripts/test-windows-pdb-verifier.ps1`, and `scripts/assert-windows-release-manifest.ps1` against the built release EXE/PDB pair

Current release facts:
- stable GitHub Release workflow reacts to pushed tags matching `v*`
- the stable release workflow validates that the tag matches `vX.Y.Z`, that `Cargo.toml` version matches `X.Y.Z`, and that `changelogs/vX.Y.Z.txt` exists before building
- the stable Windows build job restores Rust cache, runs `cargo fmt --all -- --check`, `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test --features windows --bin cpu-affinity-tool`, runs `scripts/test-windows-crash-reports.ps1` and `scripts/test-windows-tuning-recovery.ps1`, builds `cpu-affinity-tool.exe` plus `cpu_affinity_tool.pdb` with `scripts/build-windows-release.ps1`, verifies the EXE/PDB basename, GUID, and age with `scripts/assert-windows-pdb-matches.ps1`, and then verifies the built exe manifest resource with `scripts/assert-windows-release-manifest.ps1` in the same runner before upload
- the stable release publish job runs on `ubuntu-24.04` and publishes `cpu-affinity-tool.exe` plus `cpu_affinity_tool.pdb`; missing declared release files fail the publish step
- `windows-release-artifacts` is a temporary handoff between the stable Windows build and publish jobs with explicit 7-day retention; GitHub Release assets retain the published EXE/PDB. After the handoff expires, a failed publication requires rerunning all jobs to rebuild and upload it.
- stable release target: `x86_64-pc-windows-msvc`
- Linux beta prerelease workflow reacts to pushed tags matching `linux-beta-v*`
- the Linux beta prerelease workflow runs on `ubuntu-24.04`, installs the Linux GUI build dependencies, runs `cargo fmt --all -- --check`, `cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test --features linux --bin cpu-affinity-tool-linux`, and then builds `cpu-affinity-tool-linux`
- the Linux beta prerelease workflow validates that `Cargo.toml` version matches the `X.Y.Z` part of the tag and that `changelogs/linux-beta-vX.Y.Z-N.txt` exists
- the Linux beta prerelease workflow publishes `cpu-affinity-tool-linux-x86_64`, `cpu-affinity-tool-linux-x86_64.tar.gz`, and `SHA256SUMS.txt` with `prerelease: true`
- installer packaging, AppImage, Flatpak, code signing, winget, choco, and similar distribution steps are currently absent

Additional release facts:
- `changelogs/*.txt` are maintained manually
- the stable GitHub Release workflow uses `changelogs/vX.Y.Z.txt` as the release body for the matching tag
- the Linux beta prerelease workflow uses `changelogs/linux-beta-vX.Y.Z-N.txt` as the prerelease body for the matching tag
- release notes no longer rely on `generate_release_notes: true`
- `scripts/build-windows-release.ps1` sets `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` for the Windows production build; the shared release profile plus the Linux beta artifact set and debug-information policy are unchanged
- `scripts/assert-windows-pdb-matches.ps1` uses the Windows `dbghelp` API to require the expected PDB basename and matching EXE/PDB CodeView GUID and age before publishing
- `scripts/assert-windows-release-manifest.ps1` reads the built Windows exe `RT_MANIFEST` resource and asserts `requireAdministrator` plus `uiAccess=false`; UAC prompt behavior remains manual smoke validation
- `scripts/test-windows-crash-reports.ps1` builds a debug binary with the non-shipping `diagnostics-test-controls` feature and verifies pre-Tokio main-thread panic plus synthetic native-loop-error report ordering, completion markers, event kinds, and exit codes
- `scripts/test-windows-tuning-recovery.ps1` uses nonshipping `tuning-test-controls`, requires the three named recovery tests to exist, and runs the isolated native subprocess suite serially. Its Rust fixtures own temporary metadata and scalar targets; it does not certify canonical ProgramData, elevated cross-user or boot continuity.
- manual pre-release validation lives in `docs/release-checklist.md` and its subordinate `docs/release-smoke-matrix.md`
- manual Linux beta pre-release validation lives in `docs/linux-beta-release-checklist.md`
- `docs/release-process.md` documents the current automated stable tag-release flow, Linux beta prerelease flow, and their current artifact limits
- version truth is split across Git tag, `Cargo.toml`, and `changelogs/`
- that version sync is still manual before tagging, then the stable and Linux beta workflows validate the relevant tag, `Cargo.toml`, and changelog inputs

Release-impacting artifacts:
- `build.rs`
- `app.manifest`
- `assets/icon.ico`
- `scripts/assert-windows-release-manifest.ps1`
- `scripts/build-windows-release.ps1`
- `scripts/assert-windows-pdb-matches.ps1`
- `scripts/test-windows-pdb-verifier.ps1`
- embedded resources
- release workflow definitions

Privilege model:
- Windows release binary embeds `app.manifest` with `requestedExecutionLevel=requireAdministrator` and `uiAccess="false"`
- Windows file imports use the root-scoped Shell path described above instead of OLE across the elevation boundary. Native smoke must verify ordinary Explorer imports into the exact chosen group without launching the target; headless/same-integrity tests alone do not certify that external path. The existing release smoke scenario remains blocking if it fails.
- Windows CI and the stable release workflow verify the built release exe's embedded manifest resource after `cargo build --release --features windows --bin cpu-affinity-tool`
- local test/debug binaries do not embed the administrator manifest so CI and local `cargo test` can run without elevation
- `tuning-test-controls` is an explicit nonshipping root/os_api feature for development and lab tuning probes; isolated fixtures and default preflight do not certify canonical ProgramData, elevated cross-user or reboot behavior

Debug MCP diagnostics also remain behind the same debug-only fences. The feature
enables the existing opt-in render profiler; `debug_performance.rs` adds a bounded
4096-frame/61-stage-bucket/64-resource cache fed by its existing writer.
`cpu_performance` reads it off the GUI thread without native polling or repaint,
keeping inclusive stages, prior-frame backend timing, gaps and eviction explicit.
`cpu_quality` uses request-only `cpu_load/history_debug.rs` reads to distinguish
retained delivery/load quality from current exact tracked PID availability;
finer historical native causes are not invented. `debug_wait.rs` defines new
completed original sample, committed monitoring settings, worker-confirmed
release and next settled UI-frame predicates. Only an explicit pending
`cpu_wait` schedules 100 ms checks, bounded to 1–6000 ms, without full graph
serialization; timeout returns observed failure. No collector, native sampling
loop, dense PID history or shipping code is added. The stdio server advertises
fifteen tools. The same debug-only `cpu_state` exposes immutable System Tweaks
card/operation/confirmation snapshots without a native read or raw target/journal
payload. Existing `cpu_input` reaches registered System Tweaks section, scrolling,
saved-EXE, Apply/Undo, Help, PC-check and display-confirmation widgets. Those
ordinary guarded UI requests can change Windows settings during explicitly
authorized tests; the bridge adds no direct native-write RPC, new worker or
production authority. Ordinary release compilation removes both state encoding
and control IDs. Test state isolation does not virtualize Windows settings.
Windows CI runs the adapter's standard-library protocol tests, including UTF-8
stdio under a legacy Windows output encoding, and the headless debug-MCP tests
with the explicit nonshipping features. The adapter writes UTF-8 bytes directly;
the console code page cannot change JSON-RPC reply encoding.

## AGENTS.md maintenance rules
Update `AGENTS.md` in the same change when you alter:
- architecture or module ownership
- repository structure
- runtime flow
- state schema or CPU preset mechanics
- `os_api` boundaries
- platform support claims
- build scripts, manifest, or resource behavior
- important dependency roles
- CI or release process
- workflow protocol, stage rules, or review protocol documented here

Truthfulness rules:
- do not claim Linux stable releases, installers, AppImage, or Flatpak artifacts exist when they do not
- do not claim CI runs tests unless committed workflows actually do
- do not claim changelogs automatically feed GitHub Releases unless that is wired
- do not claim full cross-platform parity
- do not claim Linux runtime parity with the Windows release path

Release sync rule before pushing a release tag:
- Git tag
- `Cargo.toml` version
- matching `changelogs/*`
- release and platform facts in `README.md` and `AGENTS.md`

Tag discipline:
- use stable tags like `vX.Y.Z` only for the Windows stable release workflow
- use Linux beta tags like `linux-beta-vX.Y.Z-N` only for the Linux beta prerelease workflow

Language rules:
- all code comments must be in English
- any text intended to be committed to Git should be in English
- internal local-only operational docs may be English or Russian, but must stay truthful and consistent
- keep complex validation conditions in focused, meaningfully named methods; split independent checks into named predicates and use brief comments to explain identity, bounds, or recovery requirements

## Repo-specific workflow deltas
This repository's staged-workflow protocol is self-contained in root `AGENTS.md`. A global `AGENTS.md` may add general collaboration rules, but it is not a task-state source for this repository.

See the `Repo workflow contract` section above for the canonical workflow facts and restrictions for this repository.

Extra repo-local notes:
- local roadmap content may be written in Russian
- legacy root `ROADMAP.md` and `ROADMAP_PROMPTS.md` remain local compatibility wrappers only
- if repo-specific workflow facts change, update this file in the same change

## What this document must let a new engineer answer
A new engineer should be able to learn from this file alone:
- which binaries exist
- what layers exist and who owns what
- how persisted state and CPU presets work
- where the platform boundary is
- which local verification commands matter
- what the current release contract is
- what the canonical repo workflow artifacts are
- what the repo test-first development contract is
- when `AGENTS.md` must be updated
- where the staged-workflow protocol and optional local task handoffs are defined
