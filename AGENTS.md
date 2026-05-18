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
- Windows is the primary released and explicitly supported platform
- Linux code exists as a CI-validated desktop beta path from source for `x86_64` `glibc` desktop sessions on `X11` or `Wayland`
- Linux also has a separate beta prerelease artifact contract under `linux-beta-v*` tags, but it is not part of the stable release contract
- the project must not be described as a fully cross-platform desktop app

## Repository map
Key directories:
- `src/` - application runtime code and entrypoints
- `src/app/shell/` - the top-level `eframe::App` shell, route enums, transient UI sessions, typed shell events, and presenter module ownership
- `src/app/features/` - bounded feature modules for `rules`, `execution`, `preferences`, `topology`, and `diagnostics`
- `src/app/adapters/` - seams for persisted state loading, OS helpers, and installed-app discovery
- `src/app/runtime/` - thin composition-root state facade kept around `AppState`
- `src/app/models/` - persisted schema, domain and runtime-independent data types, CPU preset and meta helpers, `LogManager`, and running-app tracking structures
- `src/app/models/app_state_storage/` - internal persistence modules for state path resolution, storage I/O, migrations, and schema refresh; `app_state_storage.rs` remains the public storage schema and API entrypoint
- `libs/os_api/` - platform boundary for OS-specific operations; Windows internals are split under `libs/os_api/src/windows/`, while Linux remains a single-file desktop beta backend
- `assets/` - icon, screenshot, `cpu_presets.json`, and social-preview guidance
- `docs/` - release/process documentation and user-facing comparison/rationale references
- `.github/workflows/` - CI and GitHub Release automation
- `changelogs/` - manual release notes

Important root files:
- `Cargo.toml` - package metadata, binaries, features, dependencies
- `LICENSE` - MIT license
- `build.rs` - Windows resource embedding and rebuild hooks
- `app.manifest` - embedded Windows manifest with elevated privilege model
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
- `docs/github-metadata.md` - manual GitHub UI metadata plan
- `CPU_SCHEME_INSTRUCTION` - format contract for `cpu_presets.json`

## Runtime architecture
Layers:
- `shell` owns the top-level `eframe::App`, tray/window lifecycle, route enums, UI sessions, presenter dispatch, and repaint policy
- `features` own product behavior:
  - `rules` owns group and rule mutations plus logical `GroupId` / `RuleId` identity
  - `execution` owns launch, runtime tracking, reconcile loops, package-owner claims, and typed monitor notifications
  - `preferences` owns theme and monitoring toggles
  - `topology` owns CPU model/thread detection helpers
  - `diagnostics` owns startup logging and typed diagnostic event shape
- `adapters` isolate storage loading, OS helper calls, and installed-app discovery
- `models` hold persisted schema plus domain and runtime-adjacent value types
- `runtime` is now a thin composition-root facade around `AppState`

Current runtime split:
- `shell::App` owns shell-only lifecycle state:
  - `tray_rx`
  - Windows tray icon guard
  - Windows `HWND`
  - hidden-window flag
- `runtime::AppState` is the composition root over:
  - `persistent_state`
  - `rules`
  - `ui`
  - `runtime`
  - `log_manager`
- `shell::UiSession` owns transient UI-only state:
  - active route
  - group form session
  - rule editor session
  - dropped files
  - installed app picker session and cached catalog
  - rotating tip state
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

Windows runtime flow:
1. Entry point creates GUI and runtime environment.
2. `tokio` runtime is created.
3. The process lowers its own priority to `BelowNormal`.
4. `App::new` creates `AppState`, seeds in-memory logical identities, writes startup diagnostics, starts execution monitors, captures `HWND`, runs autorun once, and initializes tray integration.
5. `App::update` handles tray events, monitor notifications, hidden-window flow, file drops, applies theme, and renders the active view.

Linux entrypoint now reaches the shared `shell::App` shell, startup logging, autorun, and monitor wiring, but it still must not be described as having tray, taskbar, or focus parity with Windows runtime behavior.

## Concurrency model
- GUI runs on the main thread
- background tasks use `tokio`
- tray commands flow through `tray_rx` owned by `shell::App`
- monitor notifications flow through typed `ShellEvent` messages in `monitor_rx` owned by `RuntimeRegistry`
- persisted state uses `Arc<RwLock<AppStateStorage>>`
- running-process tracking uses `Arc<TokioRwLock<RunningApps>>`
- installed-package runtime metadata cache and ownership state use in-memory `Arc<RwLock<...>>`
- Windows tray integration uses tray-icon and muda event handlers instead of a polling loop

Background loops:
- running-process rediscovery and retracking loop
- affinity and priority verification and optional correction loop

Hidden-window flow:
- when the window is hidden, `shell::App` schedules repaint with `ctx.request_repaint_after(...)` and skips rendering
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
- current persisted schema version: `7`
- schema `v5` and older formats are dual-read and normalized in memory without eager rewrite on load
- schema `v6` and older path-target app rules receive an in-memory one-time compatibility backfill that adds the primary executable filename to `additional_processes` when no normalized equivalent already exists
- schema `v7` treats an empty `additional_processes` list as intentional user state and does not re-add the primary executable filename on load
- the upgrade from pre-`v6` data or `v6` data to the current schema happens only on an explicit save path
- before the first current-schema save after loading pre-`v6` state, persistence creates an additional `state.json.pre-v6`, `state.json.pre-v6-1`, and so on backup series
- loading `v6` for upgrade to `v7` does not create a `pre-v6` backup
- after the first current-schema save, downgrade to an older binary that only understands earlier state is unsupported
- backup rotation uses `state.json.old`, `state.json.old1`, `state.json.old2`, and so on
- persistence loading is split into `state_path`, `storage_io`, `migrations`, and `schema_refresh`

Key entities:
- `CoreGroup` - CPU core group plus assigned apps
- `AppToRun` - application launch configuration with `Path` or `Installed` launch targets
- `GroupId` / `RuleId` - logical persisted identities for groups and rules
- `AppRuntimeKey` - opaque runtime-only identity derived from `AppToRun` for tracking and monitor lookups
- `RunningApp` / `RunningApps` - tracked live processes
- `RulesContext` - logical identity catalog and index projection over persisted groups and rules
- `CpuSchema`, `CpuCluster`, `CoreInfo` - logical CPU layout description
- `LogManager` - in-memory runtime log and history

Important contract facts:
- `additional_processes` in `AppToRun` is the persisted backing field for the user-visible Tracked Process Names list and participates in runtime process matching
- path-target app rules use their visible primary executable process name for exact process-name plus image-path verified tracking; other visible tracked process names are exact user-controlled fallback matches
- `AppToRun` path targets store both source path and resolved executable path
- `AppToRun` installed targets store Windows `AUMID` and do not expose user-editable args in the current contract
- runtime tracking identity keeps the existing stable encoded key contract, but it now flows through typed `AppRuntimeKey` instead of raw `String` keys across runtime core
- logical group and rule ownership persist in `AppStateStorage.rule_identities` starting with schema `v6`
- when pre-`v6` state is loaded, `AppState` seeds in-memory logical identities immediately and persists them on the next explicit save
- the Windows `Find Installed` picker is a launch-safe Windows subset backed by `AppsFolder + Start Menu shortcuts + App Paths`, not a full OS inventory
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
- opening the active data directory in the platform file manager
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
- `AppsFolder + Start Menu shortcuts + App Paths` installed app discovery and AUMID activation
- runtime-only package metadata lookup and package-local helper tracking for installed targets
- richer process inspection
- embedded manifest and resources
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
- no Windows-style installed-app activation, AUMID identity, or package metadata parity
- `os_api` is not symmetric between Windows and Linux
- no Linux stable release artifacts, installer packaging, AppImage, Flatpak, or parity with the Windows stable release contract

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
- `cargo test --features windows --bin cpu-affinity-tool`
- `cargo fmt --all -- --check`
- `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`
- `cargo build --release --features windows --bin cpu-affinity-tool`
- `cargo test --features linux --bin cpu-affinity-tool-linux`
- `cargo build --release --features linux --bin cpu-affinity-tool-linux`

`cargo make`:
- local developer automation wrapper around tasks like `fmt`, `lint`, `build-release`, `check`, and `release`
- not the release source of truth
- CI and GitHub Release workflows do not rely on `cargo make` as the truth source

Current CI facts:
- runners:
  - `windows-latest` for the Windows release-path job
  - `ubuntu-latest` for the Linux desktop beta job
- `.github/workflows/ci.yml` cancels superseded runs per branch or PR, restores Rust build cache, keeps the Windows release-path checks on `windows-latest`, and verifies the Linux beta binary on `ubuntu-latest`
- tests are part of the committed CI contract for `ci.yml`
- the Windows CI job validates the feature-gated Windows binary path explicitly with `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --features windows --bin cpu-affinity-tool`, and `cargo build --release --features windows --bin cpu-affinity-tool`

Current release facts:
- stable GitHub Release workflow reacts to pushed tags matching `v*`
- the stable Windows build job restores Rust cache, runs `cargo fmt --all -- --check`, `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test --features windows --bin cpu-affinity-tool`, and then builds `cpu-affinity-tool.exe` with `cargo build --release --features windows --bin cpu-affinity-tool` in the same runner before upload
- the stable release workflow publishes `cpu-affinity-tool.exe`
- stable release target: `x86_64-pc-windows-msvc`
- Linux beta prerelease workflow reacts to pushed tags matching `linux-beta-v*`
- the Linux beta prerelease workflow runs on `ubuntu-latest`, installs the Linux GUI build dependencies, runs `cargo fmt --all -- --check`, `cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test --features linux --bin cpu-affinity-tool-linux`, and then builds `cpu-affinity-tool-linux`
- the Linux beta prerelease workflow validates that `Cargo.toml` version matches the `X.Y.Z` part of the tag and that `changelogs/linux-beta-vX.Y.Z-N.txt` exists
- the Linux beta prerelease workflow publishes `cpu-affinity-tool-linux-x86_64`, `cpu-affinity-tool-linux-x86_64.tar.gz`, and `SHA256SUMS.txt` with `prerelease: true`
- installer packaging, AppImage, Flatpak, code signing, winget, choco, and similar distribution steps are currently absent

Additional release facts:
- `changelogs/*.txt` are maintained manually
- the stable GitHub Release workflow uses `changelogs/vX.Y.Z.txt` as the release body for the matching tag
- the Linux beta prerelease workflow uses `changelogs/linux-beta-vX.Y.Z-N.txt` as the prerelease body for the matching tag
- release notes no longer rely on `generate_release_notes: true`
- manual pre-release validation lives in `docs/release-checklist.md` and its subordinate `docs/release-smoke-matrix.md`
- manual Linux beta pre-release validation lives in `docs/linux-beta-release-checklist.md`
- `docs/release-process.md` documents the current automated stable tag-release flow, Linux beta prerelease flow, and their current artifact limits
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
