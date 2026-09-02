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
- the project must not be described as a fully cross-platform desktop app

## Repository map
Key directories:
- `src/` - application runtime code and entrypoints
- `src/app/shell/` - the top-level `eframe::App` shell, route enums, transient UI sessions, typed shell events, and presenter module ownership
- `src/app/features/` - bounded feature modules for `rules`, `execution`, `preferences`, `shortcut`, `topology`, and `diagnostics`
- `src/app/adapters/` - seams for persisted state loading, OS helpers, and installed-app discovery
- `src/app/runtime/` - thin composition-root state facade kept around `AppState`
- `src/app/models/` - persisted schema, domain and runtime-independent data types, CPU preset and meta helpers, `LogManager`, and running-app tracking structures
- `src/app/models/app_state_storage/` - internal persistence modules for state path resolution, storage I/O, migrations, and schema refresh; `app_state_storage.rs` remains the public storage schema and API entrypoint
- `libs/os_api/` - platform boundary for OS-specific operations; Windows internals are split under `libs/os_api/src/windows/`, while Linux remains a single-file desktop beta backend
- `assets/` - icon, screenshot, bundled Inter UI font and license, `cpu_presets.json`, and social-preview guidance
- `design/` - standalone HTML design reference for the approved compact Inter-based interface; it loads only repository-local assets and is not runtime input
- `docs/` - release/process documentation and user-facing comparison/rationale references
- `scripts/` - committed release/build verification helper scripts
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
- `docs/github-metadata.md` - manual GitHub UI metadata plan
- `CPU_SCHEME_INSTRUCTION` - format contract for `cpu_presets.json`

## Runtime architecture
Layers:
- `shell` owns the top-level `eframe::App`, tray/window lifecycle, route enums, UI sessions, presenter dispatch, and repaint policy
- `features` own product behavior:
  - `rules` owns group and rule mutations plus logical `GroupId` / `RuleId` identity
  - `execution` owns launch, process-instance revalidation, runtime tracking, reconcile loops, package-owner claims, and typed monitor notifications
  - `shortcut` owns saved-rule desktop shortcut service, shortcut filename allocation, OS adapter seam, and user-safe shortcut creation errors
  - `preferences` owns theme and monitoring toggles
  - `topology` owns CPU model/thread detection helpers
  - `diagnostics` owns startup logging, typed diagnostic event shape, bounded local crash-report formatting/capture, safe report discovery/retention, report-list state, and the Windows-only read-only Event Log lookup
- `adapters` isolate storage loading, OS helper calls, and installed-app discovery
- `models` hold persisted schema plus domain and runtime-adjacent value types
- `runtime` is now a thin composition-root facade around `AppState`

Current runtime split:
- `shell::App` owns shell-only lifecycle state:
  - `tray_rx`
  - Windows local shortcut-forwarding server and primary guard
  - Windows tray icon guard
  - Windows `HWND`
  - terminal tray-quit latch
  - hidden-window flag
- `runtime::AppState` is the composition root over:
  - `persistent_state`
  - `rules`
  - `ui`
  - `runtime`
  - `log_manager`
  - `crash_reports`
  - Windows-only `windows_event_log`
- `shell::UiSession` owns transient UI-only state:
  - active route
  - group form session
  - rule editor session
  - rule editor shortcut creation result/status
  - dropped files
  - installed app picker session and cached catalog
  - crash-report delete confirmations and the last report action message
  - Windows Event Log preference action error state
- `features::rules::RulesContext` owns logical `GroupId` / `RuleId` allocation, index projection, and persisted `rule_identities`
- `features::execution::RuntimeRegistry` owns runtime process tracking:
  - `running_apps`
  - runtime-only installed-package metadata cache for Windows installed targets
  - package-owner claims for shared package-local helper processes
  - cached app statuses
  - `monitor_rx`
- runtime process identity stays keyed by opaque `AppRuntimeKey`, but tracked app ownership now also stores logical `GroupId` / `RuleId`
- shell presenters are owned under `shell::presenters`; their source files still live under `src/app/views/` via path-based module ownership
- workers emit typed `shell::events::ShellEvent` messages and do not hold `egui::Context`
- Windows crash-report discovery uses an initial/on-demand single-flight standard thread plus at most one coalesced follow-up refresh; egui rendering reads only the last completed snapshot and synchronizes its newest validated report into the retained Activity context; the Linux beta does not start this worker or expose the Crash reports route
- Windows Event Log diagnostics uses a separate single-flight worker. It is never started by construction, focus gain, tray restore, or periodic refresh: after the first rendered UI frame it can run one bounded lookup when the persisted enabled-by-default preference allows it, with at most one delayed retry after a successful empty result.

Windows runtime flow:
1. Entry point parses startup arguments into a narrow startup intent; normal GUI startup remains the default, while `--run-rule <group-id> <rule-id>` is accepted as a saved-rule startup intent.
2. Windows entry point prepares the local shortcut-forwarding endpoint before creating `tokio` or `eframe`:
   - normal GUI startup may claim the primary guard before GUI startup, but it never forwards, exits, blocks on an IPC lock, or becomes a global single-instance launch
   - `RunRule` startup first tries to claim the primary guard; if another primary owns it, the process forwards a typed `RunRule` command over the local IPC pipe and exits with the typed forwarding result code
   - if `RunRule` claims the primary guard, it cold-starts the GUI, starts the forwarding server after the shell owns the command receiver and GUI wake path, then skips normal autorun and dispatches only the requested saved rule
   - if the primary guard exists but the pipe is not ready, forwarding retries briefly and then exits non-zero instead of becoming a second primary
3. After startup forwarding selects `RunGui`, the entrypoint resolves the active crash-report directory, installs the main-thread panic hook, and prepares the startup phase before creating `tokio` or `eframe`; forwarding-only processes do not install the hook, and crash-report discovery or retention never runs synchronously on this startup path.
4. `tokio` runtime is created. A main-thread panic from this point is written best-effort without changing Rust panic semantics; background-thread/task panics delegate to the previous hook without creating a crash report.
5. The process lowers its own priority to `BelowNormal`.
6. The Windows entrypoint marks the crash-report phase as UI-running and enters `eframe::run_native`.
7. The Windows entrypoint creates `App` without dispatching startup intent, which creates `AppState`, seeds in-memory logical identities, writes startup diagnostics, starts execution monitors, captures `HWND`, initializes command-only tray integration, and starts the bounded crash-report index refresh.
8. The Windows entrypoint installs the prepared shortcut-forwarding runtime with the GUI wake callback before dispatching startup intent.
   - normal GUI startup then runs autorun once
   - `RunRule` startup then skips normal autorun and dispatches only the requested saved rule
   - if a `RunRule` cold start claimed the primary guard but cannot start the forwarding server, the requested saved rule is blocked and logged instead of launching without an owned forwarding endpoint
9. If `run_native` returns `Err`, the entrypoint synchronously writes a typed native-loop report before preserving exit code `1`, then marks the report phase as closing.
10. `App::logic` handles tray events, monitor notifications, local forwarded shortcut commands, initial/on-demand crash-report refresh polling, the gated Windows Event Log worker, focus-gain refresh, hidden-window flow, file drops, and theme application; tray callbacks only enqueue typed restore/quit commands and request repaint, while `App::logic` owns all `HWND` restore/focus/taskbar work and requests an orderly root-viewport close for Quit. Once Quit is latched, no further shell work or rendering occurs before eframe shutdown. `App::ui` renders the active view; the Event Log worker may be armed only after that first rendered frame.

Linux entrypoint now reaches the shared `shell::App` shell, startup logging, autorun, and monitor wiring, but it still must not be described as having tray, taskbar, or focus parity with Windows runtime behavior.

## Concurrency model
- GUI runs on the main thread
- background tasks use `tokio`
- tray callbacks only enqueue typed commands and wake egui; `tray_rx` is owned and drained by `shell::App` on the GUI thread, which exclusively owns `HWND` operations and the terminal Quit transition
- Windows local shortcut-forwarding requests flow through a shell-owned named-pipe server thread into `shell::App`, with per-request reply channels; request enqueue wakes the `egui` context for prompt draining
- `AppForwardingRuntime` clears the GUI wake callback and requests server shutdown during drop. It joins promptly when the worker reaches a terminal I/O completion; otherwise the detached reaper retains the pipe listener, in-flight `OVERLAPPED` buffers, and primary guard until it can safely release them. A replacement process therefore cannot claim the endpoint while that worker still owns it.
- monitor notifications flow through typed `ShellEvent` messages in `monitor_rx` owned by `RuntimeRegistry`
- persisted state uses `Arc<RwLock<AppStateStorage>>`
- running-process tracking uses `Arc<TokioRwLock<RunningApps>>`
- installed-package runtime metadata cache and ownership state use in-memory `Arc<RwLock<...>>`
- Windows tray integration uses tray-icon and muda event handlers instead of a polling loop; their process-global callbacks are installed once for the GUI process lifetime, never capture or operate on `HWND`, and retain only the typed command sender plus repaint context
- crash-report capture uses immutable precomputed context, a thread-local recursion guard, and a non-blocking process-wide writer guard; the hook does not take app-state or GUI locks

Background loops:
- running-process rediscovery and retracking loop
- affinity and priority verification and optional correction loop
- bounded Windows crash-report discovery/retention worker with no overlapping replacement worker after timeout; completion is polled briefly after explicit refresh, while a timed-out worker does not force perpetual high-frequency repaint
- separate bounded Windows Event Log worker with no overlapping replacement after timeout; disabling diagnostics cancels the delayed retry and uses a generation token to discard an in-flight result

Hidden-window flow:
- forwarded shortcut commands are drained in `App::logic` before the hidden-window render skip
- when the window is hidden, `shell::App::logic` schedules repaint with `ctx.request_repaint_after(...)` and `App::ui` skips rendering
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
- current persisted schema version: `10`
- schema `v5` and older formats are dual-read and normalized in memory without eager rewrite on load
- schema `v6` and older path-target app rules receive an in-memory one-time compatibility backfill that adds the primary executable filename to `additional_processes` when no normalized equivalent already exists
- schema `v7` treats an empty `additional_processes` list as intentional user state and does not re-add the primary executable filename on load
- schema `v9` stores `windows_event_log_diagnostics_enabled`; the pre-release schema-v8 disclosure field is ignored, and v8 and older files are effective enabled without an eager rewrite
- schema `v10` stores each rule's `manage_descendants` policy. Version-aware loading materializes missing values in pre-v10 state as the previous behavior (`true`); missing values in v10, schema-less rule inputs, and new rules use the safer `false` default.
- the upgrade from pre-`v6` data or `v6` data to the current schema happens only on an explicit save path
- before the first current-schema save after loading pre-`v6` state, persistence creates an additional `state.json.pre-v6`, `state.json.pre-v6-1`, and so on backup series
- loading `v6`, `v7`, `v8`, or `v9` for upgrade to `v10` does not create a `pre-v6` backup
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
  - the local crash-report context is runtime-only and separate from chronological entries; Activity's **Clear** action preserves it
- local Windows crash reports are separate from `AppStateStorage` and do not change schema `v10`:
  - directory: `<active-data-dir>/crash-reports/`
  - event kinds: main-thread panic and native UI-loop error
  - maximum complete report size: 256 KiB; maximum payload section: 8 KiB
  - only complete recognized UTF-8 reports with the format-v1 completion marker enter the visible index
  - newest 20 complete reports are retained; incomplete, corrupt, unrelated, nested, and reparse entries are never removed automatically
  - each panic-path writer refuses a new report after it observes 64 managed complete or partial entries; this is a non-transactional multi-process safety ceiling rather than a strict quota, so simultaneous GUI writers may exceed it slightly
  - no crash-report scan or deletion is added to the synchronous startup path; a successful normal background refresh prunes complete reports to 20, while incomplete or invalid entries require user review
  - after a completed background scan, Activity shows the newest validated report's type, timestamp, reason, and full-report path; the report file remains the complete support artifact
  - reports are never uploaded automatically and can contain local paths or system details
- Windows Event Log diagnostics is separate from crash reports and remains runtime-only, owned by `WindowsEventLogManager` rather than `LogManager`:
  - it reads only recent local `Application Error` Event ID 1000 records from the local Application log after the first rendered frame when the enabled-by-default preference remains on
  - it uses an exact executable-name plus fully-qualified-path match and accepts false negatives rather than filename-only matches
  - only record ID, UTC time, exception code, a sanitized faulting-module basename, and strictly validated optional module version, faulting offset, and process creation time may enter the retained Activity context; no raw XML, event payload, path, clipboard, state.json, crash report, dump, or automatic upload is used
  - an Activity record is unverified supplemental evidence, not proof of a previous launch or crash cause
  - Activity can disable the lookup persistently; disabling clears the visible record immediately, while a failed save is reported as session-only. The app never creates dumps or changes WER/registry settings.

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
- Windows local named-pipe and mutex transport for saved-rule shortcut forwarding
- process launch
- installed-app discovery and activation on Windows
- installed package metadata lookup on Windows
- opening the active data directory in the platform file manager
- opening crash-report directories and selecting report files through an Explorer shell process whose token is verified non-elevated and below high integrity; the pre-existing Activity data-folder action retains its direct Explorer launch contract
- bounded read-only lookup of matching local Windows Application Event Log records for enabled-by-default diagnostics
- process-instance token lookup used to prevent retained/reused PIDs from being managed as a different process instance
- Windows affinity/priority application labels the failed Win32 operation and, after an access-denied settings open, may read `ProcessProtectionLevelInfo` through a query-limited handle to explain a confirmed protected-process denial; it never changes protection or privileges
- resolving the current elevated token's per-user Desktop directory for Windows shortcut creation
- Windows shortcut creation for saved-rule launch shortcuts
- affinity read and set
- priority read and set
- process inspection and process-tree logic
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
- crate-root public shape remains intentionally narrow: external callers still interact through `OS` plus small boundary value types such as `PriorityClass` and `ShortcutSpec`
- Linux backend remains a single-file minimal backend and is not forced into parity with the Windows internal layout

Windows release-path surface:
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
- no Windows Event Log diagnostics capture, worker, or Activity context
- no Windows-style installed-app activation, AUMID identity, or package metadata parity
- `os_api` is not symmetric between Windows and Linux
- no Linux stable release artifacts, installer packaging, AppImage, Flatpak, or parity with the Windows stable release contract

## Dependencies and tooling
Only list materially relevant dependencies by actual role.

Primary runtime and build dependencies:
- `eframe` / `egui` - desktop GUI
- `tokio` - background runtime
- `windows` - Win32 bindings for shell integration, process/runtime operations, local IPC, security descriptors, and manifest/resource-adjacent Windows APIs
- `tray-icon` - Windows tray integration
- `rfd` - file dialogs
- `serde` / `serde_json` - persisted JSON schema
- `regex` - CPU preset matching and related helpers
- `once_cell` - lazy initialization
- `num_cpus` - logical thread-count detection
- `image` - tray and resource image decoding
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
Local verification commands:
- `cargo test --manifest-path libs/os_api/Cargo.toml`
- `cargo test --features windows --bin cpu-affinity-tool`
- `cargo fmt --all -- --check`
- `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/build-windows-release.ps1`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-pdb-verifier.ps1 -ExePath target/release/cpu-affinity-tool.exe -PdbPath target/release/cpu_affinity_tool.pdb`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/assert-windows-release-manifest.ps1 -Path target/release/cpu-affinity-tool.exe`
- `pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-crash-reports.ps1`
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
- `.github/workflows/ci.yml` cancels superseded runs per branch or PR, restores Rust build cache, keeps the Windows release-path checks on `windows-latest`, runs feature-gated real-binary crash probes, reproduces the stable line-table release build, verifies the built EXE/PDB identity and manifest resource, and verifies the Linux beta binary on `ubuntu-24.04`
- tests are part of the committed CI contract for `ci.yml`
- the Windows CI job validates the feature-gated Windows binary path explicitly with `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --features windows --bin cpu-affinity-tool`, `scripts/build-windows-release.ps1`, `scripts/test-windows-pdb-verifier.ps1`, and `scripts/assert-windows-release-manifest.ps1` against the built release EXE/PDB pair

Current release facts:
- stable GitHub Release workflow reacts to pushed tags matching `v*`
- the stable release workflow validates that the tag matches `vX.Y.Z`, that `Cargo.toml` version matches `X.Y.Z`, and that `changelogs/vX.Y.Z.txt` exists before building
- the stable Windows build job restores Rust cache, runs `cargo fmt --all -- --check`, `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test --features windows --bin cpu-affinity-tool`, runs `scripts/test-windows-crash-reports.ps1`, builds `cpu-affinity-tool.exe` plus `cpu_affinity_tool.pdb` with `scripts/build-windows-release.ps1`, verifies the EXE/PDB basename, GUID, and age with `scripts/assert-windows-pdb-matches.ps1`, and then verifies the built exe manifest resource with `scripts/assert-windows-release-manifest.ps1` in the same runner before upload
- the stable release publish job runs on `ubuntu-24.04` and publishes `cpu-affinity-tool.exe` plus `cpu_affinity_tool.pdb`; missing declared release files fail the publish step
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
- Windows CI and the stable release workflow verify the built release exe's embedded manifest resource after `cargo build --release --features windows --bin cpu-affinity-tool`
- local test/debug binaries do not embed the administrator manifest so CI and local `cargo test` can run without elevation

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
