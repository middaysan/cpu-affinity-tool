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
- local user-facing roadmap content may be written in Russian
- repo `workflow_mode`: `staged-default`

Overlay rules:
- `.codex/AGENTS.md` may exist only as a local additive overlay
- it may not contradict facts or restrictions from this file
- it may only tighten workflow activation through `workflow_override: inherit | explicit-only`
- it may not weaken repo-shared policy

Roadmap identity rules:
- stages use immutable `stage_id` values such as `S00`, `S01`, `S02`
- display order numbers are convenience only
- once `.codex/ROADMAP.md` records a roadmap mutation, `stage_id` becomes the canonical stage reference
- legacy root `ROADMAP.md` and `ROADMAP_PROMPTS.md` may exist as ignored local convenience files, but they are not canonical workflow artifacts

Freshness rules:
- root `AGENTS.md` is always reread at session start and on `Status`
- if assistant detects drift between this file and repo reality, it must flag the conflicting section and carry the tracked update in the next relevant repo change
- `.codex/ROADMAP.md` owns only stages, statuses, deferred items, residual risks, freshness metadata, and append-only roadmap-change history

## Project and platform status
`cpu-affinity-tool` is a desktop utility for managing CPU affinity and process priority.

Repository binaries:
- `cpu-affinity-tool` - primary Windows binary
- `cpu-affinity-tool-linux` - feature-gated Linux entrypoint

Current platform reality:
- Windows is the primary and only explicitly supported, CI-validated, published platform
- Linux code exists as a partial and experimental backend
- Linux is not part of the current release contract
- the project must not be described as a fully cross-platform desktop app

## Repository map
Key directories:
- `src/` - application runtime code and entrypoints
- `src/app/views/` - egui rendering and screen composition
- `src/app/navigation/` - route enums for active window and view selection
- `src/app/runtime/` - `eframe::App` shell, `AppState` facade, `UiState`, `RuntimeRegistry`, startup wiring, monitor loops, commands, tray and window lifecycle, and view dispatch
- `src/app/models/` - persisted schema, domain and runtime-independent data types, CPU preset and meta helpers, `LogManager`, and running-app tracking structures
- `src/app/models/app_state_storage/` - internal persistence modules for state path resolution, storage I/O, migrations, and schema refresh; `app_state_storage.rs` remains the public storage schema and API entrypoint
- `libs/os_api/` - platform boundary for OS-specific operations; Windows internals are split under `libs/os_api/src/windows/`, while Linux remains a single minimal backend file
- `assets/` - icon, screenshot, and `cpu_presets.json`
- `docs/` - release and process documentation, including the current checklist
- `tests/` - external tests
- `.github/workflows/` - CI and GitHub Release automation
- `changelogs/` - manual release notes

Important root files:
- `Cargo.toml` - package metadata, binaries, features, dependencies
- `build.rs` - Windows resource embedding and rebuild hooks
- `app.manifest` - embedded Windows manifest with elevated privilege model
- `Makefile.toml` - local developer automation wrapper
- `README.md` - user-facing project description
- `docs/release-checklist.md` - manual checklist for the current Windows-only release contract
- `docs/release-smoke-matrix.md` - compact manual smoke reference subordinate to the release checklist
- `CPU_SCHEME_INSTRUCTION` - format contract for `cpu_presets.json`

## Runtime architecture
Layers:
- `views` render UI, keep local UI-shell interactions such as file dialogs and hit-testing, and emit app intents through `AppState`
- `navigation` holds route enums
- `runtime` owns orchestration, the top-level `eframe::App` shell, `AppState`, monitor wiring, commands, tray and window lifecycle, and view dispatch
- `models` hold persisted and domain data plus reusable runtime-adjacent data structures

Current runtime split:
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
  - installed app picker state and cached catalog
  - rotating tip state
- `runtime::RuntimeRegistry` owns runtime process tracking:
  - `running_apps`
  - runtime-only installed-package metadata cache for Windows installed targets
  - package-owner claims for shared package-local helper processes
  - cached app statuses
  - `monitor_rx`
- runtime process identity is keyed by opaque `AppRuntimeKey`, not ad-hoc raw strings
- `runtime::commands::*` own use-case logic for groups, apps, launch, and preferences
- `runtime::monitors::*` own the two background monitor loops

Windows runtime flow:
1. Entry point creates GUI and runtime environment.
2. `tokio` runtime is created.
3. The process lowers its own priority to `BelowNormal`.
4. `App::new` creates `AppState`, writes startup diagnostics, starts monitor tasks, captures `HWND`, runs autorun, and initializes tray integration.
5. `App::update` handles tray events, monitor notifications, hidden-window flow, file drops, applies theme, and renders the active view.

Linux entrypoint is still much thinner and must not be described as having parity with Windows runtime behavior.

## Concurrency model
- GUI runs on the main thread
- background tasks use `tokio`
- tray commands flow through `tray_rx` owned by `runtime::App`
- monitor notifications flow through `monitor_rx` owned by `RuntimeRegistry`
- persisted state uses `Arc<RwLock<AppStateStorage>>`
- running-process tracking uses `Arc<TokioRwLock<RunningApps>>`
- installed-package runtime metadata cache and ownership state use in-memory `Arc<RwLock<...>>`
- Windows tray integration uses tray-icon and muda event handlers instead of a polling loop

Background loops:
- running-process rediscovery and retracking loop
- affinity and priority verification and optional correction loop

Hidden-window flow:
- when the window is hidden, `runtime::App` schedules repaint with `ctx.request_repaint_after(...)` and skips rendering
- the hidden-window path no longer sleeps on the UI thread

## State and data contracts
State split:
- `AppState` is the runtime facade over persisted state, transient UI state, runtime registry, and logs
- `AppStateStorage` is the persisted JSON schema

Persisted state facts:
- `state.json` path is derived from `current_exe()`, so persisted state follows the binary directory
- current persisted schema version: `5`
- older formats are migrated on load
- backup rotation uses `state.json.old`, `state.json.old1`, `state.json.old2`, and so on
- persistence loading is split into `state_path`, `storage_io`, `migrations`, and `schema_refresh`

Key entities:
- `CoreGroup` - CPU core group plus assigned apps
- `AppToRun` - application launch configuration with `Path` or `Installed` launch targets
- `AppRuntimeKey` - opaque runtime-only identity derived from `AppToRun` for tracking and monitor lookups
- `RunningApp` / `RunningApps` - tracked live processes
- `CpuSchema`, `CpuCluster`, `CoreInfo` - logical CPU layout description
- `LogManager` - in-memory runtime log and history

Important contract facts:
- `additional_processes` in `AppToRun` participates in runtime process matching and is not only UI metadata
- `AppToRun` path targets store both source path and resolved executable path
- `AppToRun` installed targets store Windows `AUMID` and do not expose user-editable args in the current contract
- runtime tracking identity keeps the existing stable encoded key contract, but it now flows through typed `AppRuntimeKey` instead of raw `String` keys across runtime core
- the Windows `Find Installed` picker is a Start-backed subset, not a full OS inventory
- tracked Windows installed targets now use a runtime-only package metadata cache plus package-local PID enrichment while the target stays tracked
- package-local helper PID ownership for multiple installed targets in the same package follows `first active target wins`
- `AppStateStorage` may rebuild `cpu_schema` for the current machine through presets when the stored schema is generic or outdated for the detected CPU model
- `LogManager` keeps a bounded in-memory chronological history with three retention classes:
  - `Regular` capped at 1000 entries
  - `Important` capped at 200 entries
  - `Sticky` retained outside normal rotation for startup and critical diagnostics

CPU presets:
- `assets/cpu_presets.json` is a compile-time source file
- presets are embedded into the binary through `include_str!`
- changing `assets/cpu_presets.json` requires a rebuild
- `CPU_SCHEME_INSTRUCTION` defines the preset format and editing rules

Data source separation:
- `state.json` - runtime state
- `assets/cpu_presets.json` - compile-time embedded source
- `changelogs/*.txt` - manual release notes, not runtime input

## Platform boundary
`libs/os_api` is the main boundary between the app and the OS. It covers:
- process launch
- installed-app discovery and activation on Windows
- installed package metadata lookup on Windows
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
  - `scheduling`
  - `processes`
  - `shell`
  - `launch`
  - `window`
  - `cpu`
- crate-root public shape remains intentionally narrow: external callers still interact through `OS` plus `PriorityClass`
- Linux backend remains a single-file minimal backend and is not forced into parity with the Windows internal layout

Windows release-path surface:
- tray integration
- taskbar and focus behavior
- `.lnk` and `.url` parsing
- registry-based URI resolution
- Start-backed installed app discovery and AUMID activation
- runtime-only package metadata lookup and package-local helper tracking for installed targets
- richer process inspection
- embedded manifest and resources
- Windows-only CI
- current published release artifact

Linux backend surface present in repo:
- `/proc`-based process inspection
- `.desktop` parsing
- `xdg-mime` URI lookup
- affinity and priority via `nix` and `libc`

Linux gaps:
- no tray parity
- no focus parity
- no runtime wiring parity
- no `Find Installed` parity
- `os_api` is not symmetric between Windows and Linux
- no Linux CI
- no Linux release artifacts
- no end-to-end validated Linux support claim

## Dependencies and tooling
Only list materially relevant dependencies by actual role.

Primary runtime and build dependencies:
- `eframe` / `egui` - desktop GUI
- `tokio` - background runtime
- `mimalloc` - global allocator
- `windows` - Win32 bindings
- `tray-icon` - Windows tray integration
- `rfd` - file dialogs
- `serde` / `serde_json` - persisted JSON schema
- `regex` - CPU preset matching and related helpers
- `once_cell` - lazy initialization
- `num_cpus` - logical thread-count detection
- `image` - tray and resource image decoding
- `winres` - Windows resource embedding at build time
- `libs/os_api` - local platform abstraction crate

Linux-only backend dependencies inside `libs/os_api`:
- `nix`
- `libc`
- `errno`
- `shlex`

Do not invent dependency purpose just because a crate appears in `Cargo.toml`.

## Build, verification, CI, and release
Local verification commands:
- `cargo test --manifest-path libs/os_api/Cargo.toml`
- `cargo test`
- `cargo fmt --all -- --check`
- `cargo clippy -- -D warnings`
- `cargo build --release`

`cargo make`:
- local developer automation wrapper around tasks like `fmt`, `lint`, `build-release`, `check`, and `release`
- not the release source of truth
- CI and GitHub Release workflows do not rely on `cargo make` as the truth source

Current CI facts:
- runner: `windows-latest`
- `.github/workflows/ci.yml` cancels superseded runs per branch or PR, restores Rust build cache, and runs `cargo fmt --all -- --check`, `cargo clippy -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test`, and `cargo build --release`
- tests are part of the committed CI contract for `ci.yml`

Current release facts:
- GitHub Release workflow reacts to pushed tags matching `v*`
- its Windows build job restores Rust cache, runs `cargo fmt --all -- --check`, `cargo clippy -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test`, and then builds `cpu-affinity-tool.exe` in the same runner before upload
- it publishes `cpu-affinity-tool.exe`
- target: `x86_64-pc-windows-msvc`
- Linux release job is still commented out or absent from the active release contract
- installer, code signing, checksums, winget, choco, and similar distribution steps are currently absent

Additional release facts:
- `changelogs/*.txt` are maintained manually
- GitHub Release workflow does not ingest `changelogs/*.txt`
- release notes currently rely on `generate_release_notes: true`
- manual pre-release validation lives in `docs/release-checklist.md` and its subordinate `docs/release-smoke-matrix.md`
- version truth is split across Git tag, `Cargo.toml`, and `changelogs/`
- that version sync is still manual

Release-impacting artifacts:
- `build.rs`
- `app.manifest`
- `assets/icon.ico`
- embedded resources
- release workflow definitions

Privilege model:
- Windows binary embeds `app.manifest` with `requestedExecutionLevel=requireAdministrator`

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
- do not claim Linux releases exist when they do not
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
- while the workflow matches `v*` and publishes `prerelease: false`, use only stable tags like `vX.Y.Z`

Language rules:
- all code comments must be in English
- any text intended to be committed to Git should be in English
- internal local-only operational docs may be English or Russian, but must stay truthful and consistent

## Repo-specific workflow deltas
This repo follows the shared staged-workflow protocol from `C:\Users\admin\.codex\AGENTS.md`.

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
- when `AGENTS.md` must be updated
- where the shared staged-workflow protocol is sourced from for this repo
