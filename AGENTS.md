# AGENTS.md for `cpu-affinity-tool`

## Purpose
This file records the actual repository structure, platform boundaries, runtime architecture, build/release contract, and required maintenance rules.

Keep it truthful. If architecture, CI, release flow, platform scope, or important repository structure changes, update `AGENTS.md` in the same change.

## Project and platform status
`cpu-affinity-tool` is a desktop utility for managing CPU affinity and process priority.

Repository binaries:
- `cpu-affinity-tool` — primary Windows binary.
- `cpu-affinity-tool-linux` — feature-gated Linux entrypoint.

Current platform reality:
- Windows is the primary and only explicitly supported, CI-validated, published platform.
- Linux code exists as a partial/experimental backend.
- Linux is not part of the current release contract.
- The project must not be described as a fully cross-platform desktop app.

## Repository map
Key directories:
- `src/` — application runtime code and entrypoints.
- `src/app/views/` — egui rendering and screen composition.
- `src/app/navigation/` — route enums for active window/view; not a controller/service layer.
- `src/app/runtime/` — `eframe::App` shell, `AppState` facade, `UiState`, `RuntimeRegistry`, startup wiring, monitor loops, commands, tray/window lifecycle, and active-view dispatch.
- `src/app/models/` — persisted schema, domain/runtime-independent data types, CPU preset/meta helpers, `LogManager`, and running-app tracking structures.
- `src/app/models/app_state_storage/` — internal persistence modules for state path resolution, storage I/O, migrations, and schema refresh. `app_state_storage.rs` remains the public storage schema/API entrypoint.
- `libs/os_api/` — platform boundary for OS-specific operations.
- `assets/` — icon, screenshot, and `cpu_presets.json`.
- `docs/` — release/process documentation, including the current checklist.
- `tests/` — external tests.
- `.github/workflows/` — CI and GitHub Release automation.
- `changelogs/` — manual release notes.

Important root files:
- `Cargo.toml` — package metadata, binaries, features, dependencies.
- `build.rs` — Windows resource embedding and rebuild hooks.
- `app.manifest` — embedded Windows manifest with elevated privilege model.
- `Makefile.toml` — local developer automation wrapper.
- `README.md` — user-facing project description.
- `docs/release-checklist.md` — manual checklist for the current Windows-only release contract.
- `CPU_SCHEME_INSTRUCTION` — format contract for `cpu_presets.json`.

## Runtime architecture
Layers:
- `views` render UI and emit user intent.
- `navigation` holds route enums.
- `runtime` owns orchestration, the top-level `eframe::App` shell, `AppState`, monitor wiring, commands, tray/window lifecycle, and view dispatch.
- `models` hold persisted/domain data and reusable runtime-adjacent data structures.

Stage 3 runtime split:
- `runtime::App` owns shell-only lifecycle state:
  - `tray_rx`
  - Windows tray icon guard
  - Windows `HWND`
  - hidden-window flag
- `runtime::AppState` is a facade with four owned parts:
  - `persistent_state`
  - `ui`
  - `runtime`
  - `log_manager`
- `runtime::UiState` owns transient UI-only state:
  - active route
  - group form state
  - app edit session state
  - dropped files
  - rotating tips state
- `runtime::RuntimeRegistry` owns runtime process tracking:
  - `running_apps`
  - cached app statuses
  - `monitor_rx`
- `runtime::commands::*` own use-case logic for groups, apps, launch, and preferences.
- `runtime::monitors::*` own the two background monitor loops.

Windows runtime flow:
1. Entry point creates GUI/runtime environment.
2. `tokio` runtime is created.
3. The process lowers its own priority to `BelowNormal`.
4. `App::new` creates `AppState`, writes startup diagnostics, starts monitor tasks, captures `HWND`, runs autorun, and initializes tray integration.
5. `App::update` handles tray events, monitor notifications, hidden-window flow, file drops, applies theme, and renders the active view.

Linux entrypoint is still much thinner and must not be described as having parity with Windows runtime behavior.

## Concurrency model
- GUI runs on the main thread.
- Background tasks use `tokio`.
- Tray commands flow through `tray_rx` owned by `runtime::App`.
- Monitor notifications flow through `monitor_rx` owned by `RuntimeRegistry`.
- Persisted state uses `Arc<RwLock<AppStateStorage>>`.
- Running-process tracking uses `Arc<TokioRwLock<RunningApps>>`.

Background loops:
- running-process rediscovery/retracking loop;
- affinity/priority verification and optional correction loop.

## State and data contracts
State split:
- `AppState` is the runtime facade over persisted state, transient UI state, runtime registry, and logs.
- `AppStateStorage` is the persisted JSON schema.

Persisted state facts:
- `state.json` path is derived from `current_exe()`, so persisted state follows the binary directory.
- Current persisted schema version: `4`.
- Older formats are migrated on load.
- Backup rotation uses `state.json.old`, `state.json.old1`, `state.json.old2`, and so on.
- Persistence loading is split into `state_path`, `storage_io`, `migrations`, and `schema_refresh`.

Key entities:
- `CoreGroup` — CPU core group plus assigned apps.
- `AppToRun` — application launch configuration.
- `RunningApp` / `RunningApps` — tracked live processes.
- `CpuSchema`, `CpuCluster`, `CoreInfo` — logical CPU layout description.
- `LogManager` — in-memory runtime log/history.

Important contract facts:
- `additional_processes` in `AppToRun` participates in runtime process matching and is not only UI metadata.
- `AppStateStorage` may rebuild `cpu_schema` for the current machine through presets when the stored schema is generic or outdated for the detected CPU model.

CPU presets:
- `assets/cpu_presets.json` is a compile-time source file.
- Presets are embedded into the binary through `include_str!`.
- Changing `assets/cpu_presets.json` requires a rebuild.
- `CPU_SCHEME_INSTRUCTION` defines the preset format/editing rules.

Data source separation:
- `state.json` — runtime state.
- `assets/cpu_presets.json` — compile-time embedded source.
- `changelogs/*.txt` — manual release notes, not runtime input.

## Platform boundary
`libs/os_api` is the main boundary between the app and the OS. It covers:
- process launch;
- affinity read/set;
- priority read/set;
- process inspection and process-tree logic;
- window focus/visibility helpers;
- URI/shortcut resolution;
- CPU model detection.

Windows release-path surface:
- tray integration;
- taskbar/focus behavior;
- `.lnk` / `.url` parsing;
- registry-based URI resolution;
- richer process inspection;
- embedded manifest/resources;
- Windows-only CI;
- current published release artifact.

Linux backend surface present in repo:
- `/proc`-based process inspection;
- `.desktop` parsing;
- `xdg-mime` URI lookup;
- affinity/priority via `nix` / `libc`.

Linux gaps:
- no tray parity;
- no focus parity;
- no runtime wiring parity;
- `os_api` is not symmetric between Windows and Linux;
- no Linux CI;
- no Linux release artifacts;
- no end-to-end validated Linux support claim.

## Dependencies and tooling
Only list materially relevant dependencies by actual role.

Primary runtime/build dependencies:
- `eframe` / `egui` — desktop GUI.
- `tokio` — background runtime.
- `mimalloc` — global allocator.
- `windows` — Win32 bindings.
- `tray-icon` — Windows tray integration.
- `rfd` — file dialogs.
- `serde` / `serde_json` — persisted JSON schema.
- `regex` — CPU preset matching and related helpers.
- `once_cell` — lazy initialization.
- `num_cpus` — logical thread-count detection.
- `image` — tray/resource image decoding.
- `winres` — Windows resource embedding at build time.
- `libs/os_api` — local platform abstraction crate.

Linux-only backend deps inside `libs/os_api`:
- `nix`
- `libc`
- `errno`
- `shlex`

Do not invent dependency purpose just because a crate appears in `Cargo.toml`.

## Build, verification, CI, release
Local verification commands:
- `cargo test`
- `cargo fmt --all -- --check`
- `cargo clippy -- -D warnings`
- `cargo build --release`

`cargo make`:
- local developer automation wrapper around tasks like `fmt`, `lint`, `build-release`, `check`, `release`;
- not the release source of truth;
- CI and GitHub Release workflows do not rely on `cargo make` as the truth source.

Current CI facts:
- runner: `windows-latest`
- checks: `fmt --check`, `clippy -D warnings`, `cargo build --release`
- tests are not currently part of the committed CI contract

Current release facts:
- GitHub Release workflow reacts to pushed tags matching `v*`
- publishes `cpu-affinity-tool.exe`
- target: `x86_64-pc-windows-msvc`
- Linux release job is still commented out / absent from active release contract
- installer, code signing, checksums, winget, choco, and similar distribution steps are currently absent

Additional release facts:
- `changelogs/*.txt` are maintained manually
- GitHub Release workflow does not ingest `changelogs/*.txt`
- release notes currently rely on `generate_release_notes: true`
- manual pre-release validation lives in `docs/release-checklist.md`
- version truth is split across Git tag, `Cargo.toml`, and `changelogs/`
- that version sync is still manual

Release-impacting artifacts:
- `build.rs`
- `app.manifest`
- `assets/icon.ico`
- embedded resources
- release workflow definitions

Privilege model:
- Windows binary embeds `app.manifest` with `requestedExecutionLevel=requireAdministrator`.

## AGENTS.md maintenance rules
Update `AGENTS.md` in the same change when you alter:
- architecture or module ownership;
- repository structure;
- runtime flow;
- state schema or CPU preset mechanics;
- `os_api` boundaries;
- platform support claims;
- build scripts, manifest, or resource behavior;
- important dependency roles;
- CI or release process;
- review/planning protocol documented here.

Truthfulness rules:
- Do not claim Linux releases exist when they do not.
- Do not claim CI runs tests unless committed workflows actually do.
- Do not claim changelogs automatically feed GitHub Releases unless that is wired.
- Do not claim full cross-platform parity.
- Do not claim Linux runtime parity with the Windows release path.

Release sync rule before pushing a release tag:
- Git tag
- `Cargo.toml` version
- matching `changelogs/*`
- release/platform facts in `README.md` and `AGENTS.md`

Tag discipline:
- while the workflow matches `v*` and publishes `prerelease: false`, use only stable tags like `vX.Y.Z`.

Language rules:
- All code comments must be in English.
- Any text intended to be committed to Git should be in English.
- Internal local-only operational docs may be English or Russian, but must stay truthful and consistent.

## Planning and multi-agent review protocol
General rule:
- subagents are required as a review mechanism when available, but they are not a veto mechanism;
- the main agent owns synthesis, decisions, and readiness calls;
- review loops stop at “no new material findings,” not “everyone agrees.”

When the user asks for a plan:
1. Draft the plan locally.
2. Run at least one review cycle with three review angles.
3. Synthesize and update the plan.
4. Repeat only if the previous round produced material findings.

Required plan review angles:
- architecture and boundaries;
- optimization, efficiency, operational consequences;
- overengineering, hidden coupling, maintenance risk.

Plan review limits:
- default max: 2 full multi-agent cycles;
- 3rd cycle only if a blocking unresolved risk remains;
- stop when no new material findings appear or only non-blocking comments remain.

When code changes:
1. Implement.
2. Run relevant local checks.
3. Send the code to three subagents for review.
4. Fix material findings or reject them explicitly with reasoning.
5. Re-run relevant checks.
6. Run a second review round only if the first round produced material findings.

Required code review angles:
- correctness and architecture;
- performance, optimization, and ops/regression risk;
- maintainability and overengineering risk.

Code review limits:
- default max: 2 full review cycles;
- 3rd cycle only if a blocking unresolved risk remains;
- lack of total consensus alone must not block completion.

Decision priority when reviewers conflict:
1. correctness and safety
2. regression risk
3. maintainability
4. performance
5. style

Final validation before handing off code:
- run relevant tests/linters/builds, or say exactly what was not run;
- verify the last round’s material findings were fixed or explicitly rejected;
- state residual risks and unverified areas.

Degraded-review mode:
- if a subagent is unavailable, hangs, or repeatedly gives low-signal review, try at most twice;
- then continue in degraded mode;
- explicitly record which angles were covered by subagents and which were covered manually.

## What this document must let a new engineer answer
A new engineer should be able to learn from this file alone:
- which binaries exist;
- what layers exist and who owns what;
- how persisted state and CPU presets work;
- where the platform boundary is;
- which local verification commands matter;
- what the current release contract is;
- when `AGENTS.md` must be updated;
- when multi-agent review is required and how it is bounded.
