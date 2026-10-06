# Modular Gaming Features: Platform and Integration Plan

Status: proposed design and implementation sequence; no runtime changes are implemented by this document.

Reviewed against `main` at `4bc23a817ff2a21c94b405ac40e4f66498d5a7aa` on 2026-10-06. Root `AGENTS.md` remains the repository contract. Delivery IDs below identify packages in this proposal, not canonical local stages or completion status. When implementation starts, map the accepted packages into `.codex/ROADMAP.md` without replacing existing stage identities.

## Outcome and scope

Build the common platform first, then integrate reversible Windows gaming settings as independent modules. Adding a module should require its own directory, a catalog registration, and any missing narrow OS primitive. It should not require another feature-specific branch in `AppState`, the shell, the coordinator, or persistence migrations.

The platform supplies enable/disable, capability checks, reliable operation dispatch, shared-resource ownership, verification, journaling, restoration, and common UI state. Each module owns its setting's configuration, interpretation, supported targets, Windows mapping, and tests.

This is a platform for managed settings, not a replacement for every existing bounded feature. Rules, execution, diagnostics, and shortcut launching keep their current responsibilities. Windows remains the primary supported platform; shared pure logic is testable on Linux without claiming Windows-setting parity.

Success means:

- One module directory contains the product behavior for one setting.
- Turning management off releases ownership and restores the actual baseline when it is safe to do so.
- A module can be tested with a fake platform without launching a game or modifying Windows.
- Two games, two modules, and two app instances cannot silently overwrite one another's managed resources.
- Requested, stored, effective, and externally changed states remain distinguishable.
- No setting is automatically enabled when upgrading an existing configuration.

The first release does not include dynamic DLL plugins, third-party executable dependencies, arbitrary scripts, a registry-tweak DSL, a background Windows service, or claims of measured FPS gains.

## Existing seams to preserve

| Current owner | Integration point | Constraint |
| --- | --- | --- |
| `src/app/features/execution/` | Verified tracked-process and rule-owner snapshots | Keep path/provenance validation, creation tokens, descendant policy, and installed-package ownership. |
| `src/app/features/execution/reconcile.rs` | Scheduling verification and correction | Currently checks classic affinity and priority together. CPU Sets needs a deliberate contract change, not another competing monitor. |
| `src/app/runtime/state.rs` | Construct one tuning facade and forward typed commands/results | Keep feature-specific decisions out of this facade. |
| `src/app/shell/` | One settings route, UI session, lifecycle hooks, and presenter dispatch | Rendering reads completed snapshots; OS work runs outside the egui render loop. |
| `src/app/models/app_state_storage/` | Versioned configuration persistence | Preserve schema-v10 compatibility, explicit-save migrations, legacy sidecar selection, and atomic save behavior. |
| `libs/os_api/` | Narrow Windows operations and boundary values | Keep Win32 handles, unsafe calls, registry access, and vendor FFI at the OS boundary. |
| Existing diagnostic events | Human-readable operation summaries | The bounded/coalesced monitor notification channel is not a reliable ownership or mutation-command transport. |

The current Windows GUI allows multiple normal launches. The shortcut primary guard does not make all GUI launches single-instance. The tuning writer lease must therefore be independent of shortcut IPC.

## Directory and dependency design

Proposed application layout:

```text
src/app/features/tuning/mod.rs
src/app/features/tuning/platform/
    contract.rs
    catalog.rs
    coordinator.rs
    journal.rs
    runner.rs
    state.rs
    tests/
src/app/features/tuning/profiles/
    config.rs
    sessions.rs
    tests.rs
src/app/features/tuning/modules/power_plan/
    mod.rs
    config.rs
    domain.rs
    backend.rs
    ui.rs
    tests.rs
    README.md
src/app/features/tuning/modules/mouse_acceleration/
    ...
src/app/features/tuning/modules/accessibility_hotkeys/
    ...
src/app/features/tuning/modules/process_qos/
    ...
```

These filenames are a template, not a requirement to create empty files. Small modules may keep related pieces in one file. The module's `README.md` records its Windows mechanism, capabilities, restoration scope, restart requirements, and smoke checklist.

Each module owns its configuration, pure decisions, feature-facing backend trait and real/fake adapters, optional settings editor, snapshot codec, and tests in its directory. Its real adapter delegates to `libs/os_api`; low-level reusable Win32 primitives remain in that crate, for example `windows/power.rs`, `windows/input.rs`, and `windows/scheduling.rs`. This is the one intentional cross-directory boundary: product logic is local to the module, while unsafe platform code stays in the existing OS layer.

Dependency direction:

- The shell and `AppState` use the public tuning facade.
- Profiles submit desired ownership to the platform; they do not call modules' OS adapters.
- Modules use the small platform contract and their backend boundary.
- The coordinator uses the catalog contract; it does not import concrete modules or match their IDs.
- Only the composition/catalog file knows concrete registrations. Modules never import one another.
- `os_api` never imports app configuration, egui, or the tuning coordinator.

Use statically compiled Rust modules. A narrow erased adapter may connect heterogeneous modules to the catalog, but configuration, snapshots, plans, and decisions remain typed inside each module. `serde_json::Value` is allowed at the versioned serialization boundary, not as the runtime model for every setting.

No public plugin ABI or separate crate per setting is required. Extract a crate only when a real dependency or reuse need justifies it.

## Common module contract

Final Rust signatures belong in the first implementation PR. The required responsibilities are fixed here:

| Contract part | Module responsibility | Platform responsibility |
| --- | --- | --- |
| Descriptor | Stable ID, config/snapshot versions, title, description, read-only/managed access, supported activation modes and target scopes | Reject duplicate IDs and invalid registration; build the catalog. |
| Capability probe | Read-only OS/build/driver/target checks; structured unsupported or denied reason | Cache bounded results and expose them without enabling anything. |
| Config codec and validation | Deserialize/migrate its own configuration; reject invalid values | Store versioned envelopes; preserve unknown configuration without executing it. |
| Observation | Read the owned setting fields and effective-state evidence available from the OS | Schedule reads, attach operation identity, and keep the latest completed result. |
| Preparation | Pure plan from configuration, target and observation; declare resources, exact baseline and ordered writes | Resolve ownership and validate that the plan has a recoverable snapshot before mutation. |
| Execution and verification | Execute narrowly defined writes; read back owned fields and report immediate/restart-dependent effect | Persist intent first, run steps serially, record progress and prevent success after partial failure. |
| Restoration plan | Reconstruct the original setting, including original absence; describe field-level drift | Permit automatic restoration only while owned fields still match the last confirmed write. |
| Optional editor | Render its typed configuration and return a draft/intent | Supply the card, lifecycle controls, status, errors, scope and restart messaging. |

Read/probe/prepare functions must not mutate Windows. Modules cannot bypass the runner for a write, create their own lifecycle monitor, or implement their own journal. No module writes a broad registry key or resets a whole driver profile when it owns only individual values.

Use common typed values for `FeatureId`, `ProfileId`, `OwnerId`, `OperationId`, `ResourceKey`, `TargetIdentity`, capability results, errors, and activation modes. Do not treat a module ID as a resource key: two modules touching the same underlying setting must contend for the same resource.

Target identities distinguish machine, user SID, interactive user session, process instance, PnP device, and GPU/display identity. A process target includes PID, creation token and verified provenance. Every mutating process operation revalidates the instance using the same opened handle, preserving the existing Windows contract.

Resource granularity covers the fields the module actually owns. For example, active power scheme, process scheduling policy, process priority, and process execution-speed QoS are separate resources. Classic affinity and CPU Sets share a scheduling resource so they cannot become independent simultaneous writers.

## Lifecycle, ownership, and reversible operations

Separate three concepts:

1. A module is compiled and discoverable in the catalog.
2. A user configures a desired value and an activation mode.
3. An owner actively requests that value for a resource.

Disabling a module's management removes its owners; it does not mean setting the Windows option to `false`. For boolean Windows settings, prefer an explicit `Unmanaged / On / Off` choice or a desired-value control plus `Restore previous`. Labels must describe the actual action.

Supported activation modes are explicit manual management and a verified game/rule session. A module declares which modes it supports. Reboot-dependent settings are never advertised as temporary game-session switches.

For a resource:

- The first owner captures the baseline; later owners never replace it with the app's own modified value.
- Equal requests share ownership. Removing one owner does not restore while another remains.
- In the first version, a different requested value is rejected with a visible conflict. Do not silently invent last-writer-wins or profile priorities.
- Changing the sole owner's desired value preserves the first baseline and updates the journal before writing.
- Removing the last owner triggers compare-before-restore and verification.
- A no-op request that already matches Windows is recorded without inventing a changed baseline.
- Persistent/manual ownership and session ownership use the same coordinator. They cannot create separate writers for one setting.

Acquire a multi-resource plan's claims together before its first write; reject conflicting plans without a partial acquisition. Keep claims for unresolved partial mutations until recovery completes. Read-only modules use observation dispatch without mutation ownership or journal entries.

Example: game A and game B both request power plan P. Windows plan Q is captured once. Closing A keeps P; closing B restores Q if Windows still uses the app's last confirmed P. If B requests plan R, B's power-plan request is reported as conflicting; the game itself can still launch.

An optional module failure does not terminate or block an otherwise valid game launch. Profile application can be partial, and the UI reports each module's result. Within a module, multi-step writes use compensation; failure does not produce a successful checkbox. Windows settings are not an atomic database transaction, so compensation can itself fail and must remain visible and recoverable.

Observe external drift before reapplying or restoring. New tuning modules do not silently fight Windows, another utility, or the user. Existing affinity/priority correction retains its current explicitly enabled behavior until a separately tested migration transfers ownership.

## Journal, crash recovery, and multiple instances

The recovery journal is separate from `state.json`. Configuration describes intent; the journal describes outstanding ownership and mutations. Configuration reset, failed migration, or loading defaults must never discard pending recovery data.

On Windows, use one canonical, protected recovery store under the Known Folder `ProgramData`, proposed as `CpuAffinityTool/tuning/journal-v1.json`. This is deliberately independent of the selected legacy sidecar/platform configuration path so moving the EXE or launching another copy does not create another baseline. Resolve the folder through the OS boundary, validate directory ownership/access rules, and refuse unsafe or unwritable storage. Debug tests use temporary/fake stores; non-elevated debug runs can remain read-only.

A native machine-wide writer lease protects this shared store and managed writes across normal app instances and Windows sessions. The lease is held for the writer's active tuning runtime; another instance can render its UI but cannot mutate or recover owned settings. Releasing the lease on process death permits the next writer to inspect the journal. The design must not change the normal GUI or shortcut single-instance contract.

Journal envelopes include format and module snapshot versions, operation/owner identities, actual target scope, resources, exact original state, desired state, last confirmed state, step progress, activation mode, and restart/effect status. Preserve absence, registry type and raw bytes; preserve GUIDs, lists, masks and individual driver overrides. Never store guessed Windows defaults as restoration values.

Required operation sequence:

1. Acquire the writer/resource ownership and validate target/capability/configuration.
2. Observe touched fields and prepare an ordered, recoverable plan.
3. Atomically publish and sync the prepared journal before the first OS mutation. If publication fails, do not write Windows.
4. Recheck target and the expected current owned fields immediately before each write. Abort on drift.
5. Execute each write, verify touched fields, and durably record progress before advancing.
6. Expose stored and effective state separately, retaining pending-restart or unresolved steps.
7. On release, compare current owned fields with the last confirmed app state, prepare restoration, journal its intent, restore and verify.
8. Mark complete only after successful verification; retain incomplete records for recovery.

Compare-before-write reduces races with external tools; it is not an atomic compare-and-swap for all Windows settings. Verification and visible conflict handling remain necessary.

Recovery runs under the writer lease before accepting new tuning writes. It re-observes rather than blindly replaying:

- Current fields equal the recorded baseline: the operation needs no restoration.
- Current fields equal an identifiable app-written step: offer/resume safe compensation, or reconcile a still explicitly configured manual owner.
- Current fields differ from known states: retain the record and report external change; do not overwrite it automatically.
- A process instance ended: retire its process-scoped record without touching a reused PID.
- The original verified game session is still alive after an app crash: reattach its owner before deciding to restore.
- The module/snapshot version is unavailable, data is corrupt, or the target cannot be validated: retain recovery evidence, block affected resources, and show an actionable error.

Use bounded parsing/record counts and no automatic eviction of unresolved entries. Test failure immediately before/after every durable step, including a write that happened before its completion was recorded. Do not claim power-loss durability beyond the actual filesystem guarantees.

On orderly Quit, stop accepting activation requests and drain session-owner releases through a bounded shutdown path outside rendering. Manual owners remain configured; failed or timed-out restoration remains journaled. Do not rely on a destructor as the only restoration mechanism. After forced termination, restoration may wait until the next launch; there is no instant rollback guarantee without a separate watchdog/service.

Per-user/session features must verify the intended interactive user's SID and session. The release EXE is elevated; credential-over-the-shoulder UAC can change HKCU/token ownership. The first implementation disables per-user mutations when it cannot prove the correct context, rather than modifying the other administrator account or introducing impersonation infrastructure.

## Profiles, persistence, and execution integration

Use versioned tuning configuration containing profiles, module envelopes and associations to existing logical `GroupId`/`RuleId`. Default to no association and no active module. A module-specific schema upgrade is owned by that module; the central `AppStateStorage` migration handles the tuning envelope, not every future setting.

Keep existing group/rule identity allocation and `AppRuntimeKey` encoding intact. In particular, the current runtime key includes priority; profile ownership must use logical rule and process-instance identities, not an editable name, array index, or mutable configuration string. Deleting a rule releases its owners and removes its association without reusing the ID.

Preserve unknown/disabled module configuration on a save; never treat it as an activation request. Disabling an available module still runs the restoration path for its existing owners. Unknown recovery snapshots require the recovery behavior above. Configuration is validated and saved before a new persistent/manual activation is accepted; a save failure is visible and does not produce an undisclosed persistent side effect.

Execution supplies a complete, generation-tagged snapshot of verified rule owners/process instances to the tuning session bridge. Lifecycle correctness must converge from the latest authoritative snapshot rather than depend on every start/stop notification being delivered. UI repaint/log notifications may remain bounded/coalesced; mutation requests have explicit acceptance, backpressure and terminal results.

Session activation follows verified tracking, including rediscovered games started outside this app. Do not activate from a launch click or filename alone. A launch failure creates no session owner. A session ends when its last eligible verified process instance disappears, according to the rule's existing descendant and installed-package policy. Helpers shared between installed targets must respect existing ownership claims.

Profile edits, module disable, rule deletion, retracking, owner changes, monitor-preference changes, tray hiding and shortcut cold starts all reconcile through this bridge. Disabling affinity correction must not prevent tuning session cleanup. Workers must publish state and release runtime/configuration locks before callbacks; never hold those locks over Windows writes or journal I/O.

Serialize mutation work in one bounded worker initially. UI commands carry operation/configuration generations so a stale probe/completion cannot re-enable a disabled module. A busy/cancelled state is explicit; cancellation cannot discard a snapshot after a write has started. No per-module polling loop or new service is needed for the first integrations.

## UI and diagnostics

Add one catalog-driven settings surface and a profile association control in the existing saved-rule editor. Common cards show purpose, scope, availability, desired value, activation mode, observed state, pending operation/restart, restoration action, and a concise reason when unavailable.

Status is structured, not one boolean. It must represent unsupported, permission denied, inactive, applying, verified active, partial failure, restoring, external change, recovery required, and stored-but-restart-required. A module supplies effective-state evidence only where its API permits it. Persisted registry state is not proof of lower latency or even that a driver is using it.

Most modules need only a toggle/select plus the common card. Complex IRQ/GPU modules may supply their own editor inside that frame. The shell knows only the tuning route and public commands, not a field or route for every setting. Read-only links to native Windows Settings are a valid unavailable/unsupported fallback.

Activity logs include module ID, operation ID, target kind, outcome, and failed OS operation. Reuse the existing diagnostics boundary; do not add telemetry or put complete journals/registry payloads into ordinary logs.

## Test strategy and platform gate

Follow root `AGENTS.md` TDD: write failing/characterization tests before behavior changes whenever technically possible. Keep coordinator and module decisions pure; inject feature backends, journal filesystem, lease provider, lifecycle snapshots and clock.

Foundation acceptance requires two test-only modules with different shapes: a single machine-scoped scalar and a multi-step user-scoped setting with original absence and restart status. They are not shipped tweaks. Prove registration, enable/disable, restoration and failure handling without special-casing either module in the core.

Required shared coverage:

- Duplicate IDs, unknown config versions, unknown snapshot versions, disabled defaults, and round-trip preservation.
- Same-value shared ownership, different-value conflict, sole-owner config edit, last-owner release and idempotent repeated commands.
- Journal publication failure causing zero OS writes; partial write, partial restore, recovery at every interruption point and corrupt/oversized data.
- Original absence/type/raw-byte restoration and preservation of unrelated fields.
- External drift, denied access, unsupported builds, vanished targets and reused PIDs.
- Two app instances competing for the writer lease and restart recovery from the same canonical store.
- Lost/coalesced notifications, complete-snapshot convergence, multiple verified processes per rule, rule deletion and still-running-session reattachment.
- Rapid enable/disable, stale worker results, cancellation after write, hidden-window processing and bounded Quit.
- Existing schema and affinity/priority/shortcut regression tests staying green.

Each real module supplies its own decision/backend tests and native Windows smoke steps. Fakes validate orchestration; they do not prove driver behavior. Native smoke records Windows build, privilege/user context, hardware/driver where relevant, initial/changed/restored state, and effective/restart behavior. Build-specific adapters stay unavailable until this matrix exists.

For implementation changes, run the repository's current Windows and Linux checks on their respective runners:

```bash
cargo fmt --all -- --check
cargo test --manifest-path libs/os_api/Cargo.toml
cargo test --features windows --bin cpu-affinity-tool
cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings
cargo test --features linux --bin cpu-affinity-tool-linux
cargo clippy --features linux --bin cpu-affinity-tool-linux -- -D warnings
cargo build --release --features linux --bin cpu-affinity-tool-linux
```

Preserve the Windows release build, manifest/PDB verification and crash-report checks from `AGENTS.md` and CI. Update `AGENTS.md` and the release smoke matrix in the implementation PR that actually changes architecture or behavior; do not describe this proposal as implemented.

The platform is ready for real modules only when a second test module can be added with its directory and registration, with no feature-ID branch in shared code, and all ownership/recovery tests pass.

## Delivery sequence

Keep implementation PRs focused and mergeable. Each package may be split into smaller PRs; do not combine the foundation and the full tweak catalog into one change.

| Package | Deliverable | Dependencies | Exit criteria |
| --- | --- | --- | --- |
| F1: contracts and module harness | Tuning facade, stable IDs, typed scopes/resources, catalog, two fake module/backend shapes and contribution template | Accepted design | Pure lifecycle contract tests; no Windows mutations; current behavior unchanged. |
| F2: safe runner and ownership | Serialized dispatch, baseline ownership, sharing/conflicts, step verification and structured statuses | F1 | TDD coverage for repeated requests, last release, drift and partial failure; no concrete tweak logic in the core. |
| F3: durable recovery | Canonical journal store, native writer lease, versioned snapshots, recovery and bounded shutdown | F2 | Fault injection at every write boundary; multi-instance protection; forced-termination recovery demonstrated with fake/test targets. |
| F4: app integration | Versioned profile/config persistence, verified execution snapshot bridge, common cards and rule associations, diagnostics | F3 | Old state loads with all tuning off; UI issues intents only; two fake modules work through the complete path; Linux checks pass. |
| I1: power plan | First real module: choose an existing Windows power scheme for a session | F1-F4 gate | Two-game ownership, conflict, external change, failed activation and exact GUID restoration pass on Windows. |
| I2: input conveniences | Separate mouse-acceleration and accessibility-hotkey modules, preferably one PR each | I1 validates the platform; verified user/session context | Each independently restores original fields; neither changes unrelated accessibility settings. |
| I3: process QoS | Per-process execution-speed throttling policy for explicitly selected rules | F4; process identity boundary | Original masks preserved; denial/reused-PID tests; no automatic management of all background processes. |
| I4: CPU Sets | Real topology/group enumeration, one scheduling mode per rule, CPU Sets module and reconcile-contract migration | F4; scheduling characterization tests | No competing hard-affinity writer; groups/SMT tested; CPU-set baseline and live-instance checks restored correctly. |
| I5: optional priority sessions | Transfer session-managed priority to one owner while preserving existing rule defaults/behavior | F4; explicit ownership migration | Launch/manual/monitor paths agree on one writer; disabling tuning cannot be immediately undone by the legacy monitor. |
| I6: Windows settings adapters | Separate Game Mode, background capture, windowed-game optimization and GPU-selection modules | Platform gate; supported-build smoke matrix per module | Unknown builds use native Settings; registry writes are not reported as universally effective. |
| A1+: optional advanced modules | Parking profile, HAGS, IRQ, vendor GPU settings, measurements and display profile, each separately gated | Relevant identity, restart and recovery support | Per-module acceptance below; no requirement to ship these for the platform/MVP. |

F1-F4 contain the universal foundation. I1 is the first production vertical slice and may refine a contract based on evidence; any refinement must keep both fake module shapes and shared tests valid. Freeze the small stable contract after that slice, and grow it only for demonstrated needs.

## Integration backlog and boundaries

| Module directory | Mechanism and target | Restoration and shipping boundary |
| --- | --- | --- |
| `power_plan` | `PowerGetActiveScheme` / `PowerSetActiveScheme`; existing plan, machine scope | Exact original GUID; last-owner release. Do not equate Power Plan with Power Mode or manufacture Ultimate Performance. |
| `mouse_acceleration` | `SystemParametersInfo`; verified interactive user/session | Snapshot the full mouse parameter tuple; restore owned fields. Describe pointer behavior, not FPS or a raw-input game fix. |
| `accessibility_hotkeys` | Sticky/Filter Keys hotkey flags | Preserve accessibility state and unrelated flags. Do not disable accessibility features wholesale. |
| `process_qos` | `Get/SetProcessInformation`, `ProcessPowerThrottling` | OS-managed / EcoQoS / explicitly disable execution-speed throttling; preserve original masks and independent bits. No bypass of thermal limits. |
| `cpu_sets` | Real CPU Set topology and process default sets | Restore original list; hard affinity can constrain selection and thread-selected sets can differ. Do not infer P/E class from CPU index. |
| `process_priority` | Existing `SetPriorityClass` boundary, with session ownership | Capture before the first relevant launch/monitor write or do not claim that the original is known. Default choices stay modest; no Realtime gaming preset. |
| `game_mode` | Native Settings first; narrowly verified Windows adapter later | Build/context-gated state reads/writes and exact snapshot; deprecated UWP API is not the implementation. |
| `background_capture` | Native capture setting; supported-build adapter | Disable background recording separately from Game Bar. Old GameDVR policy is not assumed universally effective on Windows 11. |
| `windowed_optimizations` | Windows graphics setting for compatible windowed/borderless DX10/11 games | Preserve per-app exceptions and other preference fields; represent game restart. Do not promise a benefit for every DX12/Vulkan game. |
| `gpu_preference` | Windows per-app preferred GPU on supported configurations | Preserve the complete original override and unrelated preference fields; distinguish requested adapter from observed renderer. |
| `core_parking` | Dedicated cloned game power plan; documented AC/DC and heterogeneous-CPU parameters | Never edit an arbitrary user's base plan. Journal clone creation/ownership; restore the prior active plan and safely clean up only the app-owned unchanged clone. Optional measured experiment. |
| `hags` | Driver-supported Windows hardware scheduling; build-verified adapter | Manual experiment only; stored/effective/reboot states separated for both apply and restore. No universal preferred value. |
| `irq_affinity` | SetupAPI/PnP interrupt policy, real processor groups | Manual advanced module; exact value presence/type/bytes, no whole-key deletion; reboot. Explain USB-controller shared devices and that IRQ policy does not pin every DPC/audio thread. Gate unsupported group layouts; MSI starts read-only. |
| `nvidia_profile` | Documented NVAPI DRS settings | Optional adapter; restore individual override/absence, not the whole profile. Driver/version capability checks. |
| `amd_3d` | Documented AMD ADLX interfaces | Optional adapter; test support and true scope per API version. GPU-wide controls must not be presented as per-EXE profiles. |
| `measurements` | Optional PresentMon or exported benchmark capture | Read-only collector with bounded lifecycle; compare frame times under reproducible conditions. No score that implies a predicted FPS gain. |
| `display_profile` | Display enumeration, then optional documented mode switch | Diagnostics first. A mode-changing version needs a tested confirmation timeout and independent restoration when the UI cannot be seen. |

For IRQ, start with inventory and observation in one PR; add mutation only after correct target/group identity and reboot recovery are proven. For vendor GPU support, start with support/read-only state and one documented setting rather than a full profile editor.

Exclude mass service shutdown, package removal, security/VBS/Defender/UAC changes, Windows Update disable, HPET/BCD/timer hacks, RAM/standby cleaners, blanket networking tweaks, unused MMCSS `GPU Priority`, and a universal `SystemResponsiveness=0` preset. These do not meet this module catalog's clear, low-impact, precisely reversible scope. MPO remains an issue-specific diagnostic investigation, not a default gaming module. Engine-integrated Reflex/Anti-Lag 2 cannot be enabled for arbitrary games by this platform.

Reuse mechanisms and UI ideas from the research through independent implementations based on official APIs. Check any copied code's license per file; do not import incompatible/noncompete code or a whole tweak preset as the source of truth.

## Adding a module after the foundation

1. Create `modules/<stable_id>/` with typed config, observation/snapshot, pure plan, backend seam, optional editor and tests.
2. Declare exact targets, resources, activation modes, capability conditions and restart semantics.
3. Implement only read/probe first; test supported, unsupported and denied outcomes.
4. Write failing tests for apply, original-state restoration, partial failure and external drift before adding writes.
5. Add a versioned config/snapshot codec and use the common runner; no local journal or separate owner loop.
6. Add the module declaration and catalog registration; add a narrow `os_api` primitive only if missing.
7. Add its Windows smoke checklist and run relevant existing regression/CI checks.
8. Review the diff for changes outside its module. Shared lifecycle changes need a demonstrated missing contract and shared tests, not a feature-ID exception.

Definition of done for a shipping module: default off, truthful support/scope, verified target identity, persisted baseline before mutation, tested exact restoration, visible failure/drift/restart states, no competing writer, and documented native smoke evidence.

## Primary references and repository context

The October 2026 gaming-tweaks research supplies the candidate list; performance gains were not measured. The following primary references identify mechanisms, not benchmark evidence:

- Repository contracts: [AGENTS.md](../AGENTS.md), [CONTRIBUTING.md](../CONTRIBUTING.md), [Windows smoke matrix](release-smoke-matrix.md), and [existing shortcut plan](shortcut-launch-plan.md).
- Microsoft: [Power scheme management](https://learn.microsoft.com/en-us/windows/win32/power/managing-power-schemes), [CPU Sets](https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets), [process power throttling](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation), and [process priority](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass).
- Microsoft: [SystemParametersInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfoa), [Sticky Keys](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-stickykeys), and [native Settings URIs](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-settings).
- Microsoft: [interrupt affinity](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/interrupt-affinity-and-priority), [DPC target selection](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-kesettargetprocessordpcex), [core parking](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/options-for-core-parking-cpmincores), and [MMCSS limitations](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service).
- Vendor APIs: [NVIDIA NVAPI DRS](https://docs.nvidia.com/nvapi/group__drsapi.html) and [AMD ADLX 3D settings services](https://gpuopen.com/manuals/adlx/adlx-sdk-references/adlx-interfaces/3d-graphics/iadlx3dsettingsservices/).
