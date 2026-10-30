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

### Verification workflow

- Select checks from the changed behavior and its failure modes. Extend an existing regression or shared fixture before creating another equivalent test.
- Every new test must protect an observable behavior, invariant or intentional build/security contract. Avoid tests of a fixture's hardcoded answer, immutable self-comparisons and incidental source formatting. Do not create production abstractions solely to increase coverage counts.
- During implementation, use the narrow failing test and affected checks. Run the applicable broader checks after the change settles; repeat them only after new changes, failures or unresolved concerns. Documentation-only edits need text/link/diff review, not an application rebuild or native probe.
- Consolidate duplicates only after identifying the retained check that would catch the same defect. Keep deterministic identity/lifetime/logon, ownership/stale-generation, CPU/RAM union/partial/unknown, persistence/conditional Undo, revocation/shutdown and shipping-fence checks. Live polling supplements these checks; it cannot replace controlled race or failure injection. There is no test-count reduction target.

Choose local validation by scope; committed CI and release gates remain mandatory:

| Change | Relevant local validation |
| --- | --- |
| Documentation and project instructions | Review accuracy, links and `git diff --check`; do not claim runtime validation from a text edit |
| Presentation only | Reuse existing layout/geometry coverage; inspect the affected view when clipping, input or scaling can change; do not add unit tests just to mirror a label or color |
| Pure logic or persisted state | Focused TDD/regressions, then affected platform tests, formatting and Clippy |
| Configuration, monitoring or production MCP | Relevant deterministic tests plus affected built-app cases from `docs/testing.md`; use the default live pack for changes crossing the writer, telemetry or HTTP/UI boundaries |
| Launch, Focus, affinity or priority | Add the relevant native readback/interactive case, including `-InteractiveLaunch` where applicable; preserve exact owned-process cleanup |
| Native System Tweaks | Controller/recovery regressions and, when authorized, exact field readback/reload/conditional Undo; real background-role promotion still requires controlled-VM qualification |
| Build, dependencies, manifests or feature fences | Affected platform/feature builds and the existing artifact, renderer, PDB, manifest, crash/recovery or vendor gates |

Use the existing tools and evidence boundaries:

- Reuse `scripts/test-windows-live-mcp.ps1` and the helpers documented in `docs/testing.md` for built-app tests. Add a case to that runner before creating an ad-hoc launcher, worker or testing framework. In the default suite, keep fixtures synthetic, autorun/enforcement/tuning Off at startup, and mutations limited to explicitly owned test data/processes. Never copy user state or recovery journals into that suite.
- Use bounded waits, terminal receipts and exact process handles/lifetimes. Always clean up owned children, listeners, tickets and temporary keys; forced or incomplete cleanup is a failure. Missing privileges, a desktop or supported counters do not count as successful coverage. This workflow grants no additional permission to change Windows settings or admit real background roles.
- Distinguish pure/state tests, a diagnostic EXE, the exact shipping EXE and interactive/vendor/VM acceptance. Debug-MCP selectors and authority overrides are absent from shipping builds; `LOCALAPPDATA` alone does not isolate production authority or Windows settings. A diagnostic pass is not shipping or real-service certification.
- Report compilation, test execution and setup/cache time separately. Compare the same source/toolchain/profile/cache conditions and identify the artifact; a cached root-test rebuild speedup is not an application or full-CI performance gain. Before calling a PR green, verify its current head; an earlier green run remains evidence only for that earlier head.

### README screenshot maintenance

- Review screenshot impact whenever visible layout, copy, navigation, controls, themes, metrics, charts or tables change. Refresh the affected README scenes in the same change; refresh the full set for shared header/font/theme/layout changes. Keep captions and links accurate. For a reviewed nonvisual source change, explain why the existing image still represents the UI; never relabel old provenance as a fresh capture.
- Follow `docs/readme-screenshots.md` and reuse `scripts/test-windows-live-mcp.ps1 -CaptureReadme` with the fixed scene contract. Capture actual settled renderer frames and measured owned workloads; do not synthesize/retouch UI or inject chart values. Use isolated synthetic rules/authority, leave real Windows settings untouched and require successful cleanup before publication.
- Use `scripts/publish-readme-screenshots.ps1` for the checked PNG whitelist and generated lead image; run `scripts/check-readme-screenshots.py`. Preserve the lead's three columns (Affinity groups / CPU selection / CPU history), original unscaled individual frames and clear exterior separators. Inspect the actual PNGs at full size and README display width; do not rely on hashes alone for visual correctness.
- Record source/artifact/scale/scenes and cleanup in `assets/screenshots/capture.json` and keep `assets/screenshots/README.md` truthful. Never commit user state, tickets, keys or raw inventories. Static integrity checks in CI do not replace capture/visual review, and a documentation screenshot is not a shipping or performance acceptance result.

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
- `src/app_icon.rs` - shared Windows ICO decoder for the root viewport/taskbar and tray; the EXE resource embeds the same `assets/icon.ico`
- `src/app/shell/` - the top-level `eframe::App` shell, route enums, transient UI sessions, typed shell events, and presenter module ownership
- `src/app/views/service_controls.rs` - compact header service controls and Process monitor modal; the shell dispatches dialogs without a bottom panel
- `src/app/shell/render_preparation.rs` - bounded Windows font preparation and the nonblocking 60 Hz CPU-modal scroll paint budget; the existing native event loop retains queued input until the deadline
- `src/app/features/` - bounded feature modules for `rules`, `execution`, `preferences`, `shortcut`, `topology`, `diagnostics`, `cpu_load`, and `system_tweaks`
- `src/app/features/execution/control/` - single-writer process arbitration, immutable conflict views and versioned executable-scoped owner preferences. The existing discovery loop supplies exact memberships; the existing state writer serializes durable owner changes with native scheduling callbacks. No second process inventory, watcher or sampler.
- `src/app/features/cpu_load/` - Windows-only read-only CPU telemetry, bounded histories, a single sampling worker and the compact group/process graph presenter. Native PDH/process-time/ETW collectors live under `libs/os_api/src/windows/cpu_load/` behind `libs/os_api/src/cpu_load.rs`.
- `src/app/features/cpu_load/process_list.rs` - current per-rule and per-group whole-host CPU totals, exact member-union deduplication and coverage cached once per sampling update; current summary widgets share one immutable snapshot per GUI frame.
- `src/app/features/cpu_load/memory.rs` - current app/group private resident RAM aggregation/formatting, sharing exact tracked identities and the existing CPU worker; no memory history or independent sampler.
- `src/app/features/cpu_load/export.rs`, `history_export.rs` and `export_ui.rs` - explicit bounded CPU/process CSV capture/download and native graph PNG save, sharing the existing history without another sampler.
- `src/app/features/rules/known_apps.rs` - Windows new-GUI-import defaults for20application families, bounded persisted provenance and pure installation-layout projection; enrichment runs only for new logical rules on the existing save worker. `libs/os_api/src/app_identity.rs` and `windows/app_identity.rs` provide bounded PE product/layout evidence, exact current-logon checks and ordinary client activation. No new watcher or sampler is added.
- `src/app/features/rules/process_names.rs` - request-driven background process-name suggestions for the rule editor, separate from enforcement and CPU collection.
- `src/app/features/mcp/` - optional production Windows localhost Streamable HTTP agent API, its compact connection UI, bounded request receipts and adapters to existing configuration, execution and monitoring features. This is separate from debug-only GUI automation.
- `src/app/features/mcp/wire_contracts.rs` - once-built input descriptions and successful output schemas; complete rule-replacement guidance is shared by the catalog, server initialization and copied agent prompt.
- `src/app/features/execution/managed_runs.rs` - exact managed child handles and optional bounded stdout/stderr drains, owned by execution independently of HTTP access.
- `libs/os_api/src/windows/agent/` - native protected MCP key storage, verified interactive-parent ordinary launch, suspended child verification and exact-instance process control.
- `src/app/features/system_tweaks/platform/` - small typed contracts/codecs, the module registry and owned-field controller
- `src/app/features/system_tweaks/config.rs` - bounded profile/configuration persistence shapes and exact forward-data retention; parsing supplies no module execution or native source authority
- `src/app/features/system_tweaks/service.rs` - candidate-state saves with logical identities and manual module requests
- `src/app/features/system_tweaks/settings_update.rs` - explicit compatible-file upgrade and recovery checks on the existing writer. The cached typed snapshot drives an inline Update saved settings notice; the shared read-only loader guards never repair or publish merely observed input. Unknown/blocked/stale files remain refused, and a completed save is distinct from native recovery readiness.
- `src/app/features/system_tweaks/execution_bridge.rs` - bounded execution/foreground observations with copy/inspect/recheck publication, scheduled refresh and coalesced refresh requests
- `src/app/features/system_tweaks/session_reducer.rs` - one bounded application worker, existing execution observations and immutable profile/card/shutdown snapshots
- `src/app/features/system_tweaks/background_policy.rs` and `background_ui.rs` - optional per-app background sessions and cached topology/diagnostics, advanced by the existing tuning worker. background_policy/rule_settings.rs maps app-rule drafts onto existing session profiles and preserves earlier shared selections. background_ui.rs renders only the Application rule checkbox; no Advanced/core picker or separate System Tweaks card remains. The module adapter lives in `tweaks/background_workload/`; native role/SCM/task/signature verification and conditional process mechanisms live in `libs/os_api/src/windows/background/` behind `libs/os_api/src/background.rs`.
- `src/app/features/system_tweaks/presentation.rs` - typed module validation/tweak metadata and transient help/undo sessions; `src/app/features/system_tweaks/view.rs` is the compact System Tweaks presenter
- The existing `tweaks/power_mode/` adapter applies only the AC power-mode preference for new tweaks. Retired CPU configurations and their exact scheme/setting bindings remain readable for recovery and Undo. `libs/os_api/src/gaming_cpu_power.rs` and `libs/os_api/src/windows/gaming/cpu_power.rs` retain their typed native restoration boundary, including exact absence restoration.
- `tweaks/core_parking/` is a separate manual Windows 11 parking adapter. `libs/os_api/src/core_parking.rs` validates bounded topology/scope snapshots; `windows/gaming/cpu_power/parking.rs` reads available AC policy fields through the existing guarded CPU-power boundary. All-core activation uses captured parking-only keys. Detected separate classes are visible but unavailable, and new separate-class configurations are rejected after host functional validation found the class-1 setting ineffective. There is no arbitrary chiplet/CPU-mask or E-only control. The existing tuning worker publishes immutable options with card observations; Apply revalidates before saving, and the shared controller retains exact legacy-field conflicts and conditional Undo.
- `src/app/features/system_tweaks/pc_checks.rs` - pure summaries, hardware guidance and explicit read-only PC checks; bounded native reads live in `libs/os_api/src/windows/pc_checks.rs`
- `src/app/adapters/` - seams for persisted state loading, OS helpers, and installed-app discovery
- `src/app/runtime/` - thin composition-root state facade kept around `AppState`
- `src/app/models/` - persisted schema, domain and runtime-independent data types, CPU preset and meta helpers, `LogManager`, and running-app tracking structures
- `src/app/models/app_state_storage/` - internal persistence modules for state path resolution, storage I/O, migrations, and schema refresh; `app_state_storage.rs` remains the public storage schema and API entrypoint
- `libs/os_api/` - platform boundary for OS-specific operations; Windows internals are split under `libs/os_api/src/windows/`, including startup-only own-process scheduling in `self_affinity.rs` and suspended-launch affinity inheritance handling in `launch.rs`; Linux remains a single-file desktop beta backend
- `libs/os_api/src/windows/file_drop.rs` - GUI-thread Shell file-drop registration for the Windows root, with window-scoped UIPI exceptions, bounded path-only payloads and captured physical release coordinates; no worker or process-wide filter
- `assets/` - icon, generated three-view README lead image, bundled Inter UI font and license, `cpu_presets.json`, and social-preview guidance; `assets/screenshots/` holds the original native frames, capture provenance and checksums. `docs/readme-screenshots.md` and the existing live MCP runner own their refresh workflow.
- `design/` - standalone HTML design reference for the approved compact Inter-based interface; it loads only repository-local assets and is not runtime input
- `docs/` - release/process documentation and user-facing comparison/rationale references
- `scripts/` - committed release/build verification helper scripts
- `vendor/eframe/` - published eframe 0.36.2 with documented initial-root-visibility, opt-in native root-frame deferral and debug-only frame attribution hooks; upstream provenance, licenses and removal criteria are in `LOCAL-PATCH.md`
- `vendor/epaint/` - published epaint 0.36.2 with the documented default variable-font hinting-coordinate equality fix; upstream provenance, licenses and removal criteria are in `LOCAL-PATCH.md`; Windows CI runs its library regressions
- `vendor/winit/` - published Winit 0.30.13 with the upstream Windows 11 mixed-DPI rectangle-policy fix backported from `488c036`; Windows 10 retains its existing adjustment. Provenance, license, regression scope and removal criteria are in `LOCAL-PATCH.md`.
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
- `docs/mcp.md` - optional production Windows HTTP MCP setup, full-access key, exact process/run control, bounded output and original-time CPU cursor contracts
- `docs/system-tweaks.md` - user guide to the current compact Windows settings, explicit Apply/Undo, persistence and read-only guides/checks
- `docs/mcp-validation.md` - artifact-specific Windows MCP acceptance, release comparisons, CPU19 diagnostic soak and remaining renderer/platform limits
- `docs/tuning-native-probe.md` - actual module apply/restore evidence, recovery checks and remaining validation limits
- `docs/background-role-qualification.md` - test-owned functional progress/readback/Undo evidence and the unexecuted controlled-VM role/function qualification protocol, including unresolved task-action identity and result-observability prerequisites
- `scripts/test-windows-background-workload.ps1` - deterministic background admission/work-protocol checks, run without native switches in Windows CI and stable release verification together with strict os_api test-code Clippy; an explicit local `-OwnedProcessProbe` adds the bounded native arithmetic-child probe, never real-role admission or service installation
- `docs/github-metadata.md` - manual GitHub UI metadata plan
- `CPU_SCHEME_INSTRUCTION` - format contract for `cpu_presets.json`

## Runtime architecture

Shared process observation and scheduling ownership are separate. The same exact PID/creation can contribute to several saved rules and groups; CPU/RAM keep their existing per-rule and union deduplication. Classic Run/Fix/autorun, reconciliation, installed post-launch correction and production MCP scheduling use one shared Authority. Existing rules win new overlaps in persisted insertion order. Legacy rules with conflicting effective fields require an explicit owner; equal settings select one quiet writer. A preferred logical RuleId is bound to a canonical executable image and participates only when it claims that instance; removing it permits eligible fallback. The Application rule editor shows potential duplicate names separately from actual shared-process conflicts, stages Take control here/owner selection until Save and discards them on Cancel. Staged ownership choices mark the editor dirty on both desktop paths. Process-name lists are never pruned by arbitration. Conflicting rows offer Resolve; observation-only rows do not offer an endless Fix. Same semantic launch keys in separate groups retain independent logical runtime entries and rekeying preserves lifetime evidence.

Deferred initial correction resolves the effective owner under that same authority gate, including launches while automatic Process monitor is Off; confirmation remains attached to the initiating rule's bounded intent, not a new continuous enforcement loop. Read-only handoff checks never create module reservations. Namespace-only runtime-key transitions preserve verified logical-rule continuity in GUI and saved MCP lookups before ordinary discovery migrates the map. Installed correction, including ordinary MCP activation with no returned PID, uses the existing bounded poll and exact same-AUMID/current-logon seeds. Package candidate evidence retains the PID/creation token across image/AUMID reads and final publication. It captures its source and validates current group/rule/target/membership configuration inside each native initial-write gate and before publication under RunningApps -> authority ordering; stale callbacks stop rather than resetting ownership metadata. Newly qualified package helpers receive a guarded initial correction after releasing roster locks, and count as new work for the bounded poll stability check. Conflict notices resolve separate exact claimant sets, including disjoint shared sets of one image. The bounded preference list retains earlier eligible owners of independent sets; the latest eligible choice wins, and editor staging uses the same ordering. Old format1 settings remain readable and upgrade only through the existing writer. No persisted process list is edited by these paths.

Periodic discovery and reconciliation capture the saved policy generation with their configuration. Before changing runtime membership or applying automatic settings, they take a nonblocking configuration observation lease and reject stale generations; reconciliation also checks the saved monitor flag. Durable publication takes the matching exclusive lease before the Authority gate, so a completed Off/save cannot be followed by an older periodic write. The order is RunningApps -> observation lease -> Authority; the writer never acquires RunningApps. Already initialized GUI actions bypass that lease. Unsupported control policy still permits read-only discovery while native scheduling remains refused. No second inventory, roster copy or worker is introduced.

State schema14 retains bounded `process_control` metadata (format2 order/legacy/ordered eligible choices/generation, with format1 read compatibility). Read-only loading initializes legacy interpretation without rewriting old groups/lists. Only the application writer publishes choices, after a durable save; failed saves retain the previous runtime owner. Unsupported/foreign ownership data blocks writes and remains retained. Older-schema ownership evidence is refused. The bounded authority gate precedes SchedulingHandoff and never acquires RunningApps or GUI state locks; discovery may acquire RunningApps before the gate. GUI reads immutable views. `read_process_control` and revision-checked `choose_process_owner` expose the same policy through the existing MCP command/receipt path. Real-role admission remains closed.

GUI save completion is identified by its own request receipt on both desktop paths, rather than the most recent worker operation. The existing worker retains at most64terminal receipts, pinning at most one GUI result until its draft owner adopts the corresponding revision. Normal dispatch and orderly shutdown use the same revision/revocation admission; revoked queued MCP work cannot become a shutdown save.

New GUI file/shortcut/installed-Path imports recognize qualified layouts for20application families and fill the existing editable process-name list once. One optional schema13 `builtin_process_binding` retains catalog ID/version and generated-name provenance. Existing rules, raw discovery/MCP create inputs, intentional empty lists and manual extras are not automatically expanded. GUI SaveBase imports tag enrichment, and the existing application writer applies it only to new logical RuleIds before durable publication. Reads, editor opens, launches and catalog updates never refill names; all rule-replacement paths preserve provenance. Unknown/incompatible data is retained but cannot globalize generated names or execute unknown launcher semantics. Older-schema binding evidence blocks reinterpretation, while ordinary v12 rules remain unchanged on upgrade. Linux retains metadata without executing Windows generated matching.

The additional18families use the static `rules/known_apps/catalog.rs` data table: finite main/ordinary-launcher/helper paths and native PE company evidence. New GUI imports seed only member names whose canonical app-owned files exist, with a128-entry numeric-version component bound; the binding reserves the catalog's known names at that import so later explicit enabling remains scoped. Main-only rows explicitly omit shared WebView2/services. EA/Battle.net/Ubisoft/Rockstar intermediate launchers reuse ordinary acknowledgement and the same one-shot discovery/readback; no new watcher, sampler, worker, schema or UI is introduced. Old rules and existing version1Steam/Discord bindings are untouched. Vendor layouts beyond the finite table require separate acceptance; owned PE fixtures are not universal real-app certification.

Generated members require a finite qualified installation layout, current interactive logon and exact PID/creation identity; the ordinary primary retains its existing exact-image match. Steam games/services and arbitrary Steam descendants are not automatic client members. Known logon checks are cached within one discovery observation (at most4096 entries), clear at the next observation and never replace final fresh lifetime checks. The shared RunningApp primary-instance witness supports launch decisions independently of the metric-selected set, considering every finite main executable name in the qualified catalog layout. Generated orphan helpers retire; explicit manual fallback semantics remain. Discord's root Discord.exe launcher and canonical Update.exe shortcut use verified ordinary launch acknowledgement without scheduling/registering the updater; actual Discord.exe versions are tracked. The existing bounded one-shot intent also serves known applications and readback while continuous monitoring is Off. CPU/RAM use the same admitted identities and existing union/dedup/coverage. Focus uses verified member windows, then ordinary selected-client activation for a hidden client, without scheduling writes or a claimed foreground guarantee. Production MCP saved built-in launches use the same ordinary activation/readback contract without inherited output capture or elevated activation; old unbound EXE rules retain managed launch. PE product/layout evidence is classification, not cryptographic publisher authentication. See docs/known-app-processes.md and the explicit owned-client native probe; real package/update compatibility requires separate app acceptance.

Steam game shortcuts have a distinct persisted `Steam` launch target (schema v12): native AppID plus an optional exact game executable. New Windows .url/.lnk Steam game imports preserve game intent for complete native AppID URI and steam.exe -applaunch AppID forms; unsupported extra arguments are refused instead of becoming Steam-client rules; legacy Path rules convert only through an explicit editor action. Launch dispatches a validated Steam URI through the verified ordinary Explorer broker and never registers/schedules the returned Steam client as a game. Existing discovery and reconciliation own live game membership and one-shot post-launch scheduling even when continuous enforcement is Off; atomic launch admission and generation-conditional failure cancellation coalesce GUI/MCP requests; launch intent is bounded to five minutes and keyed by logical rule plus AppID, surviving exact-EXE selection. Per-instance one-shot application is consumed only after native readback; late members can receive initial settings within that window, and already confirmed instances are not continually enforced while monitoring is Off. Bounded local libraryfolders/appmanifest metadata resolves configured games, and suitable game-window images seed automatic exact PID/creation membership. Raw claims include every configured Steam target, including explicit and ambiguous observations. Bounded exact plausible-main evidence is retained by logical rule/AppID independently of published membership, including hidden windows and uncertain access; positive exit/reuse retires it. Helpers also retain their role under uncertain access, and every admitted member passes the native current-interactive-logon boundary. Multiple candidate images or overlapping configured claims withdraw automatic membership and show Select game; known Steam/launcher/anti-cheat/support images are excluded. Exact retained game instances survive window minimization. Named helpers and optional descendants stay within the game root (or the explicitly selected image directory when outside the resolved Steam stub). Exact helper roles survive game exit until those lifetimes end, preventing helper-to-main reseeding. All saved activation paths use a coherent main-only Focus snapshot; native Focus ranks real main windows rather than taking the first visible window. The rule editor offers one debounced, coalesced running-process picker with query-before-cap filtering, stale-result fencing and no idle scans, retaining worker ownership across editor resets, saving the chosen executable path; it does not launch a candidate. CPU/RAM reuse the same verified membership and existing dedup/coverage without another sampler. Steam rules currently execute only on Windows; no private Steam interfaces, hooks or process-log dependency is added. URI acknowledgement is not startup proof, desired affinity from the first game instruction is not guaranteed, and protected games can refuse scheduling. Real-game coverage requires manual acceptance; native owned-process validation alone is not universal certification. MCP preserves Steam AppID/executable replacement semantics and ordinary activation acknowledgement.

Steam installation failures now remain as bounded per-rule runtime reasons rather than becoming empty successful observations. Ambiguous/missing metadata, conflicting candidates and helper-only candidates produce an amber Select game action; an undetected launch offers selection after30seconds while the initial scheduling intent remains limited to five minutes. Clicking the status or Select opens the existing editor and process picker with the reason visible. Exact selection clears the warning only after verified membership; helper lists and persisted schema stay unchanged. The existing discovery/cache/worker owns this state, with no added inventory or sampler.

Background workload control is a Windows 11 x64 local preview, Off by default. A Session-only module stores logical saved-EXE rule IDs in ordinary tuning profiles, independently of rule association or classic monitor enforcement. Application rule owns one default-Off Reduce background load checkbox, with no Advanced/core-placement controls; the standalone System Tweaks card is removed. App and policy drafts publish atomically through the existing writer, and Cancel discards both. New settings are per rule; editing a legacy shared selection splits only that rule while preserving other games and original source metadata. Unknown/ambiguous configuration cannot silently reset a policy. Plain MCP rule replacement preserves the separate background settings. No state-schema bump or extra watcher is introduced. Active background session policies are normalized to Priority/EcoQoS only; new app saves clear old placement choices for that rule without rewriting other saved rules. Reading retained configuration does not rewrite it. CPU Set codecs/witnesses and internal finite-preview/recovery contracts remain for compatibility and conditional restoration, never as hidden new placement intent. Read-only discovery reuses the existing execution bridge's process enumeration, verifies the selected image and current interactive logon, and does not insert into RunningApps. Game lifetime, rather than foreground focus, governs the session. A separate SessionTarget owner binds the exact game/preview source and exact background PID/creation token. The original field module retains a bounded native role/file/service witness through equal-owner sharing and reload; parsing the witness grants no authority, and native mutations regenerate it. Existing-process priority reservations remain distinct from fresh-launch admission and retire only after durable field retirement. Expired target evidence retires only that target; game exit, preview expiry, normal shutdown and retained-preview recovery conditionally restore live targets. Drift remains pending without overwriting external changes. No additional worker, CPU sampler or idle inventory loop is added. Applicability uses a bounded five-second lazy inventory while a session is active or one explicit check; Off with no retained ownership performs no inventory. Candidate details, session events and signature caches are bounded.

Background session sources retain the selection-time group metadata while matching current membership by logical rule ID, so moving a rule cannot create another trigger, owner or CPU pool. Native preflight retains equal desired-field intents: the controller alone skips an unowned suitable baseline or joins an already verified equal owner's original context.

Explicit saves and new profile edits reclaim only the internal single-rule background profile shape whose logical rule is gone and whose recovery ownership is known to be retired. Unknown/legacy/custom profiles and unadmitted or uncertain recovery remain intact. Orphaned selectors create no observation demand. Saved-EXE discovery uses the verified Explorer/UAC-linked desktop logon, caches exact candidate PID facts within one observation, and isolates optional access/exit failures from reliable tracked/foreground facts; optional lifetime handles do not enter the required global recheck set. The legacy profile lookup decodes once per lookup, independent of the number of per-rule policies. Background-only unsaved drafts participate in the existing shortcut dirty-state guard.

Retained game owners recheck current saved-EXE rule membership without the bounded UI choice-list limit. Removing that rule or group withdraws its owner even while the source process is still alive. For retained-policy equality, a legacy shared selector is projected to the exact source rule; editing another game's checkbox cannot withdraw an unchanged peer, while changes to the source's other policy settings still invalidate its previous request. The test-only lifetime seam is absent from ordinary builds. Durable controller regressions cover shared owners, last-owner Undo, reload, publication failure and external drift without admitting real roles.

Identity verification and native role mutation qualification are independent. The compiled mutation qualification set is currently empty: real Windows roles remain NOT ADMITTED even after signature/task/SCM verification. The local delivery is validated with test-owned processes and controller fixtures; it does not certify system-service function, crash recovery for actual roles, game benefit or other hosts. Owned-child mechanism helpers and fixtures remain behind test/nonshipping compilation fences and cannot enable production system-role mutation. Controlled VM role/function validation is required before any later promotion. Existing manual Windows tweaks retain their separate admission and recovery contracts.

The separate test-only `native_function_tests.rs` / `native_function_workload.rs` probe requires independently checked arithmetic work on its exact owned child, separate Priority/EcoQoS/combined cases, original SystemManaged/Off/On restoration, bounded pipes/deadlines and normal child exit; it has no CPU Set or system-role target. Scheduled-task RUNNING state and matching registered actions currently do not bind Appraiser/Device Census to an exact action process; this remains a pre-admission identity gap. Functional oracles for actual roles and any future mechanism/build/context-specific admission must be established in the controlled VM, not inferred from the local probe.

The optional production MCP module is compiled into the single Windows EXE and is disabled by default. One compact MCP modal opened from the header owns enable, localhost-only HTTP port, Copy connection and Copy AI prompt. One permanent bearer key gives full access to every supported tool; there are no named clients, scopes or saved-launch approvals. The official Rust MCP SDK handles Streamable HTTP; authorization additionally requires an exact loopback Host and refuses browser Origin headers. Schema-2 connection settings live in a separate administrator-owned, high-integrity `mcp/<storage-profile-sha256>/access.json` under the current account's native known application-data directory, never in the editable state schema. The payload binds its storage profile. First creation persists the key before publication; restarts, Off/On and port changes retain it. Schema-1 migration discards old tokens, generates one new key, preserves the port and forces Off. Invalid/foreign/future settings are refused without replacement. Policy loading/saving and native actions stay off the GUI thread. Off fences queued writes, closes the listener and does not terminate resumed programs or stop independent CPU monitoring. Existing debug MCP remains behind its original debug-only compilation fences.

Production MCP configuration commands use the existing single application writer, logical identities, immutable snapshots, revision checks and durable completion receipts. Request sequences and bounded receipts prevent a retry from repeating a retained or evicted action within an API epoch; changed epochs require inspection before retrying execution. CPU queries reuse the existing collector and RAM history. Each retained history row records its last mutation revision and, when delivered, native interval ID/QPC boundaries/frequency; paging can upsert delayed rows at their original time. Managed roots are distinct Run ID sources in that worker/history and sparse contributor pool, excluded from the saved tracked union and owned by the single local-agent connection. Their numeric rings and metadata enter the same 256 MiB history budget, release on monitoring Off and follow bounded run-record retention. Current CPU vectors are paged and share the response's numeric-cell bound. Historical ETW lifetime IDs are explicitly distinct from native creation tokens and cannot authorize process control. Managed EXE launches default to a verified ordinary interactive Explorer parent, with explicit privilege and scheduling readback before resume; administrator launch requires explicit elevated=true. AUMID activation reuses the verified ordinary Explorer broker and reports activation acknowledgement rather than an invented PID. Exact control verifies the same interactive logon and rejects reserved scheduling fields. Normal window close and explicit forced termination remain distinct. Output captures drain through bounded byte buffers with explicit cursor gaps and independent EOF/exit state. Application log reads copy bounded GUI snapshots; crash reads copy a validated report ticket and revalidate directory/file identity on a bounded native worker, accepting no remote path. Artifact-specific Windows acceptance and its remaining renderer/platform limits are recorded in `docs/mcp-validation.md`; this does not certify other hosts or a published release.

Headless System Tweaks resolves Apply targets on its existing application worker, independently of GUI-card visibility. Manual profile activation resolves a module-provided opaque target ID again before changing settings; process-session modules retain rule associations. Display changes use only the compact Apply/confirmation path with its existing rollback deadline. The HTTP tool catalog is one immutable allocation. Native-query admission is owned by the application rather than a listener generation: its four slots survive Off/On, port changes and cancelled HTTP callers, and an uninterruptible native closure holds its slot until completion. Scheduling handoff metadata is pruned only under capacity pressure using bounded native exact-exit probes outside its lock; alive or uncertain identities retain all reservations and attempted-write evidence. Managed-run display labels and exit summaries are bounded by Unicode count and escaped JSON bytes, keeping every retained row within the shared wire budget without changing saved names or operational IDs. On application exit, admission is revoked before the existing recovery shutdown, and Tokio receives a bounded 250 ms shutdown wait so an uninterruptible native call cannot hold the process open indefinitely.

CPU history export is explicit. `export_cpu_history` starts one bounded preparation job or reads the returned immutable artifact by UTF-8 byte cursor. CSV schema 1 uses a UTF-8 BOM, commas and CRLF; rows retain original UTC/monotonic times, CPU percentages, process PID/historical lifetime/name, rule/run scopes, original interval/QPC metadata, parking, delivery/gaps and attribution quality. CPU totals and positive sparse process contributions are separate record types; scopes must not be summed together, missing data stays blank and formula-like basenames receive a spreadsheet-safe apostrophe. The existing history supplies at most 128 copied rows per lock acquisition; encoding and file I/O run outside the sampler lock and GUI. A changing/evicted interval aborts instead of mixing pages. One preparation is admitted at a time, at most one MCP artifact is retained, files are capped at512MiB, downloads at32KiB per reply and IDs expire lazily after600seconds or replacement. The native boundary creates a random CREATE_NEW temporary file with exclusive sharing and delete-on-close, retains its exact handle and accepts no remote path. Monitoring Off does not invalidate an already captured artifact. Shutdown cancels preparation; no idle worker, cleanup timer or new CPU sampler is added.

Parked CPU tiles retain the neutral body (or the existing selected fill); only parking status text is yellow. Parking help names the displayed original sample time and distinguishes instantaneous Windows parking policy from interval load and retained mini-chart history, without changing telemetry. CPU tile and detail-plot context menus offer Save this graph (PNG) and Save all CPU history (CSV). PNG uses one explicitly requested native screenshot, crops only the visible chart region at its physical pixel scale, rejects changed scale/viewport size, and encodes/saves on a finite worker through the standard Save As dialog. CSV reuses the same bounded capture and standard dialog. Popup layers belong above their modal parent. Rule process-name fields offer at most eight case-insensitive matches, prefixes first, from a bounded on-demand system-name snapshot. Opening an editor or active typing/focus can request a refresh, coalesced behind one worker and a five-second freshness bound; there is no idle polling or per-keystroke native scan. Filtering is cached by text and snapshot, selecting a suggestion changes only the unsaved draft, and arbitrary manually entered names remain supported.

Current application rows also show private resident RAM while CPU load monitoring is enabled. The existing one-second worker queries GetProcessMemoryInfo/PROCESS_MEMORY_COUNTERS_EX2 through the already verified retained process handle and rechecks lifetime. Each exact PID/creation-token is queried once and counted once per rule, including additional tracked names and enabled descendants; shared physical pages are excluded. Current snapshots retain only per-rule sums and coverage, clear them on invalid membership/configuration or Off, and reject stale revisions. A measured subtotal is marked with an asterisk; missing/denied/unsupported counters stay unavailable. EX2 requires Windows 10 22H2 or Windows 11 22H2 with the September 2023 cumulative update or later; older counter implementations are not silently replaced with commit or shared working-set estimates. CPU and RAM share one 52-by-21-logical-pixel app-row button: the upper CPU line has proportional fill and the lower RAM line is muted. Stopped apps show one neutral dash; the whole active button opens CPU graphs. Bundled-font ink bounds are tested at 100%, 125%, 175% and 200% in both themes. Wrapped hover help describes both metrics. CPU histories, CSV and production MCP contracts are unchanged. Debug-only rendered-control metadata permits bounded native acceptance of the displayed value and is absent from ordinary release builds.

Group buttons also show current private resident RAM below CPU within their existing 78-by-30-logical-pixel header footprint. The same memory pass computes per-rule values and the exact PID/creation-token union for each group, reusing one native read per instance across all rules/groups. Shared instances count once within each group, rather than summing potentially overlapping app values. A current empty group has zero RAM; an absent invalid/stale membership snapshot stays unavailable, and unreadable instances retain partial coverage. Group totals are cleared with rule memory on configuration/membership invalidation and Off; retained map/key capacity enters runtime bookkeeping. This introduces no memory history, extra worker, scan or native query. Group CPU uses the same current exact member union and whole-host percentage scale as the app rows. Shared instances count once within each group. Coverage/invalidation and empty-group zero follow the same current snapshot. Both CPU/RAM summaries remain independent of delayed or unavailable ETW; the detailed per-CPU history retains its original source and clock. Group CPU maps and key storage enter the existing ProcessList bookkeeping. One bounded Arc snapshot is retained through each GUI frame and released on monitoring Off, preventing mid-frame publication from mixing app/group measurements; main summary paints do not acquire the history lock.

Application-row whole-host CPU load preserves the readable process-time subtotal when one tracked sibling is unavailable or unprimed. The existing worker caches per-rule sums and measured/total counts in the current ProcessList after exact owner/PID/creation-token deduplication; invalidation retires the same coverage. The GUI marks that subtotal with an asterisk and explains coverage. With no valid current measurement the value remains unavailable, never zero. Native access requirements, the complete current API total, original-time ETW group/per-CPU histories and independent RAM coverage remain unchanged. This adds no native query, worker or render-time process scan. The longest partial CPU label uses a measured width guard with an 8.5-pixel minimum within the same compact button.

The immutable MCP catalog publishes described input fields and typed successful `outputSchema` contracts for all tools. Errors remain separate `isError` code/message results. Operation receipts retain null/uncertain/completed distinctions, and module payloads remain explicitly module-owned. Complete `update_rule` replacements require all six editable settings plus exactly one launch target before operation admission and at the existing configuration-service boundary; omission cannot reset values through deserialization defaults. Read/write name mapping and full-list semantics are shared with the copied setup prompt. Windows unit tests use a pinned dev-only JSON Schema validator without network/file resolvers; it is absent from the release dependency graph.

MCP profile/association IDs are lossless decimal strings. Agent rule inputs reject Realtime before persistence, preventing later classic enforcement from bypassing the agent launch/control restriction; existing manual configurations remain readable and supported. Full key authority includes the existing elevated monitor's saved scheduling policy and exact native process APIs; ordinary launch remains the default, with explicit elevated=true for administrator launches.

MCP and Process monitor reuse the same compact service-modal presentation: a semibold title/Close, short muted purpose, Enable control with actual status, and equal-width segmented tabs. Empty bodies remain compact; populated bodies have a 260-logical-pixel target, with overall height bounded by the viewport. Connection has a Port row and paired Copy connection / Copy AI prompt actions with short visible setup/full-trust help; Logs shows the same newest-100 authenticated command journal, with actual endpoint port on the control hover. Wrapped errors precede the scroll body so a full journal cannot displace them. There is no separate log modal or client/scope editor. Port saves on focus loss/drag completion, tab change or either copy action; a typed deferred-copy choice waits for durable save/listener readiness and uses the saved permanent key and actual endpoint. Copy connection retains its JSON shape; Copy AI prompt adds self-contained client setup, read-only verification, reload limitations and future API/telemetry guidance. Credentials enter only explicit clipboard output, never visible labels or logs. Setup/trust tooltips explicitly wrap within a viewport-bounded 260-logical-pixel width. Close/Escape releases the journal view cache. The header MCP button is the sole entry and also opens this window while Off. One fixed local-agent ownership identity retains existing run/receipt fences; authenticated concurrency stays bounded to two requests and API version 2 reports full access without permission flags. The typed 100-call RAM ring stays in the existing Ledger, includes reads/validation failures/retries and follows mutation receipts to terminal outcomes; cancellation remains uncertain. Known tools have static human labels; arguments, unknown supplied tool names, keys and output are never retained. UI snapshots copy only on revision changes while Logs is visible, and UI reads use try_lock. Traffic-driven repaint requests are coalesced; the 180 ms receipt pulse requests only a bounded final repaint, with no idle timer/worker/disk log. MCP modals suppress the obscured main body and file imports through existing shell policy.

Windows uses WGPU/DX12 with `MemoryHints::MemoryUsage` and graphics validation layers disabled for ordinary operation. WGPU backend dependencies are Windows-only; Glow/OpenGL dependencies are Linux-only. scripts/test-renderer-dependencies.py checks both resolved normal dependency graphs in Windows CI. Both themes share the same text rasterization options, installed with the bundled fonts before first-frame warmup. Warmup processes at most 32 glyphs per pass and requests a 60 Hz cadence. The local epaint patch treats equivalent default variation coordinates as equal without changing nondefault invalidation or atlas pixels.

CPU history cut selection never mutates the canonical exact PID sampler inventory. A transient membership/lock race uses a separate reused runs-only scratch vector for that cut; the following valid cut resumes saved-app targets without requiring another membership change. Original uncertain intervals stay gaps, and later positive app/group/history/contributor/current-API values use the existing worker and original-time histories. The scratch capacity enters runtime bookkeeping.

The own process remains BelowNormal on its last representable logical CPU. After Tokio creation, only the GUI thread receives relative priority +1 with guarded readback/restoration; workers retain relative priority zero. Contributor history keeps an 8192-slot `u16` identity index in an ordinary aligned eight-byte entry, retaining f32 loads, lifetime identity and existing capacity. Memory estimates use the actual entry size. Debug-only native frame probes require explicit opt-in and reuse the existing bounded profiler; callbacks, clocks and probe module are absent from ordinary release builds.

The main navigation is Affinity groups / System Tweaks / Logs on Windows; System Tweaks remains absent on Linux. The group list has no repeated heading; New group stays at the top right of its pinned toolbar. The top row contains App settings on the left and a right-aligned service cluster. Windows uses 21-pixel-high MCP / Process monitor / CPU load controls with nine-pixel Inter Medium labels and preferred widths 50/86/82 logical pixels, separated by six-pixel gaps (230 pixels total). Longer names receive more width; constrained space shrinks the cluster rather than widening the window. Linux retains one 86×21 Process monitor control at the right. The application version is the last read-only App settings row; its hover shows the CPU model and logical thread count. There is no bottom panel, monitoring island or reserved footer height. The application title appears only in the native window title bar.

`src/app/views/service_controls.rs` owns the compact button presenter and Process monitor modal/table; the header owns button placement, while the shell dispatches service modals in the unchanged diagnostic Footer bucket without reserving any panel. MCP and CPU feature presenters expose their control buttons independently of their window location. Full service names remain in accessibility, tooltips and modal titles; the complete visible labels fit the compact row. Native control IDs, service lifecycles and modal background-paint/import suppression are unchanged.

The service controls share Run's five-pixel corners and single soft cast shadow with a subdued neutral-gray fill. Text is gray while Off, green while MCP/process monitoring is ready or CPU recording is enabled, and flashes red for 180 ms on authenticated MCP tool receipt or drained process-monitor events. Borders blend the strong neutral border with the status color at 35 percent. Opening any button never toggles the service. CPU load keeps its title at the left and a right-aligned percentage over its proportional-load fill while enabled; Off shows only the neutral CPU load title. Full service names remain in tooltips, accessibility metadata and modal titles. The native button solely owns body/frame/focus/shadow, and its CPU overlay adds only fill/text; both themes and supported scale tests keep 100% inside the compact clip. CPU controls contain Enable, history duration, memory estimate, bounded wrapped errors and Open CPU graphs; that action closes controls and opens System scope with all CPUs and processes. App settings starts with a single Theme / Light / Dark / System selector row, followed by startup controls and the version, with no duplicate heading or service entries. Service help explicitly wraps within a viewport-bounded 260-pixel width, including CPU errors and long monitor cells. The Process monitor button opens the existing shared Process monitor modal with Enable process monitor and Processes / Logs tabs. Processes retains the same compact sortable roster; Logs shows at most 100 recent retained monitor events from the existing LogManager, filtered by typed source rather than message text. Existing monitor warnings, queue-loss notices and routine messages keep their original retention classes, and saved enable/pause transitions are logged only when the existing writer's state is adopted. Logs starts no roster refresh, sampler or worker; CPU-load recording stays independent. Opening/closing these windows does not alter settings.

The main navigation fills the available header width with 28-logical-pixel buttons. Selected main tabs use a small theme-derived rounded gradient mesh, 120 ms selection transitions, 80 ms hover and immediate pressed feedback; normal buttons retain hit testing, selection semantics and keyboard focus. Existing egui animation state owns the transitions, zero style animation time disables motion, and settled decoration requests no repaint. Panel content changes immediately and stays static. A two-pixel content-facing rail slides to the selected tab with one existing egui scalar animation (at most 120 ms); zero animation time snaps and settled state requests no repaint. The System Tweaks section switch is centered, capped at 300 logical pixels and uses smaller flat 20-pixel buttons, keeping the primary navigation visually dominant.

Explicit in-app Close actions share a 44×21 logical-pixel button with theme-specific dark-red tokens and standard focus/hover/pressed behavior. The CPU graph has one Close at the top right. Native application Run buttons retain their 42×21 logical-pixel target and action dispatch, using five-pixel corners, an opaque dark-green fill in both themes, readable pale text, a neutral border matching the application-row background, one soft native cast shadow behind the body with a smooth five-pixel falloff and no inset contour strokes. Hover/pressed/focus remain native; pressed reduces the outer shadow. Fix, Focus and unavailable actions keep their existing presentation; this decoration adds no timer or repaint. Group cards show their name without ordinal badges; their unchanged logical Group ID is read-only in Edit. Group and application names own a full-width stable drag target even when the text is short; separate drag grips are absent. Focused group names retain Up/Down keyboard reordering. CPU load controls use a 21-pixel-high summary filling the compact header CPU button, with its short title and percentage inside the same bar. The native status button solely owns the body/frame/focus/shadow; the shared CPU painter adds only proportional fill and text, with a stable measured 100% reserve beside the title. It creates no nested button or focus target. The Affinity groups toolbar contains only its description and New group; it stays fixed outside the bounded affinity-groups-scroll viewport while only group cards scroll. The header summary averages the existing PDH samples across every logical CPU. Open CPU graphs in its controls opens the shared CPU modal with an explicit System target, all CPUs and all host processes by default, without a saved-group identity. Group cards retain a 70×13.5 logical-pixel load pill and a muted Group CPU load caption below it inside one subtle 78×30 outlined button between the title and Edit. The entire block shares one click/focus/tooltip target and existing control ID, and fits the existing 32-pixel title row. Both summary bars reuse the same load_meter painter with rounded inner fill clipped at the measured load and right-aligned percentages. The header bar follows the five-pixel status-button corners and9.5-pixel text; group summaries retain pill corners and nine-pixel text; application rows use a compact 44×18 percentage between status and Run/Fix. No duplicated all-process lane or separate row meter remains inside cards. Current group/application summaries share whole-host process-time measurements; partial * evidence, status colors, unknown/zero and click/focus semantics are preserved. Detailed graphs retain original-time per-CPU scheduler residency. Group CPU summaries retain every CPU number for layout-only truncation and full hover text; the title reserves the measured Edit width and optional meter. Group/app meter widths are reserved before sizing their draggable names; disabled monitoring reserves no meter space. Application rows reserve only the cached compact status galley width plus two pixels and reuse that galley for right-aligned painting, returning the former proportional status reserve to names. The name/status gap is eight logical pixels; other controls keep four-pixel gaps and full status/PID tooltips. Stopped application graph meters use a faint neutral border, row-like fill and muted dash while retaining their explanation and noninteractive state. Native imports and internal reorder targets intersect the visible scroll clip so occluded cards cannot consume a drop beneath the pinned toolbar. There is no new native read, worker, history allocation or repaint timer. Decoration is static and requests no repaint.

Layers:
- `shell` owns the top-level `eframe::App`, tray/window lifecycle, route enums, UI sessions, presenter dispatch, and repaint policy
- `features` own product behavior:
  - `rules` owns group and rule mutations plus logical `GroupId` / `RuleId` identity
  - `execution` owns launch, process-instance revalidation, runtime tracking, reconcile loops, package-owner claims, and typed monitor notifications
  - `shortcut` owns saved-rule desktop shortcut service, shortcut filename allocation, OS adapter seam, and user-safe shortcut creation errors
  - `preferences` owns theme and monitoring toggles
  - `cpu_load` owns read-only Windows CPU graphs, independently of affinity/priority enforcement. One worker reuses 1 Hz system CPU samples for the overall summary and exact PID/creation-token process totals for current app/group summaries. While CPU monitoring is enabled, one owned real-time scheduler trace collects tracked exact instances and host process contributions, independently of modal visibility or tray hiding. Process start/rundown/end metadata supplies bounded executable basenames and independent lifetime identities, including processes which start and exit between polls; no command lines, SIDs or kernel addresses are retained. New instances join shared accounting without restarting it; PID/token changes retire only the affected slot. Disable and orderly shutdown stop the trace; an empty saved/tracked set still records host contributors. Trace errors/loss and stale samples are unavailable, not zero. The worker rechecks tracked-membership generation before publishing process results, merges runtime instances by logical rule and samples shared instances once. A pinned system CPU summary and compact inline group/application meters present CPU load; process meter color retains settings compliance even at zero/unknown load, and clicking its percentage or bar opens the graph. Disabled CPU monitoring hides all load meters and their spacing; the existing text status remains. Modal charts use monotonic geometry, binary-search original-sample hover and local HH:MM:SS labels. Tessellated line/parking meshes and clock labels are cached by data, selected CPU, size, theme and pixel scale; pointer movement reuses immutable meshes.
    The system scheduler stream correlates a process end with the QPC time at which its pinned native lifetime was positively alive. Ends older than that observation cannot retire a reused PID. An admitted exact instance retains its final scheduler interval after confirmed exit and becomes known zero only in wholly subsequent intervals, including CPUs never scheduled by that dead instance. Target updates retain its ended accounting slot until ownership removes it, and new TIDs cannot charge that retired slot. Native checks stay outside the callback lock; trace loss, uncertain lifetime, startup and undelivered boundaries remain unavailable. The existing runtime memory estimate includes slot metadata. A 2026-10-16 elevated read-only browser/controlled-child probe reached complete samples for38selected instances on20logicalCPUs, retained the final exited-child work and then measured zero; this is a host probe, not universal gap-free coverage.
    The group button's percentage/fill uses the current process-time sum of unique exact instances assigned to that group, including additional names and enabled descendants, on the same whole-host scale and frame snapshot as app rows. It does not read historical ETW or selected-CPU capacity. Per-CPU group graphs retain their original-time tracked-union source and explicit selected logical CPU scale. The single pinned system summary uses the complement of the documented PDH Idle Time counter across all logical CPUs; this overall metric is not repeated within groups. Modal system graphs use non-idle scheduler residency from the same original interval as their contributor rows. Interrupt accounting and whole-machine process percentages remain different metrics; unattributed scheduler work is explicit partial attribution. The same worker reuses read-only CPU Set topology to sample Parked flags once per second, mapping by processor group/logical index; missing or conflicting flags remain unknown. Original cut parking observations stay with their interval even when its load completes later. Load gaps remain gaps, never proof of parking. Disabling stops these reads too; no parking policy is changed.
    Event-time history uses original QPC/wall-clock cuts registered by the worker. Before applying a later delivered event, the callback splits scheduler residency exactly at each queued cut, closes the old target membership and then admits the next exact PID/token set. Sixteen preallocated cut/frame buffers bound delayed numeric storage; they are intervals, not a guaranteed number of seconds. The first cut establishes a baseline. Original rule/name/lifetime ownership snapshots stay attached to pending rows. Delayed completion fills that exact retained provisional row and its sparse contributors under one history writer; eviction, real loss, unknown schemas, out-of-order timestamps, pool overflow and restart never redirect work into a newer row. Lost rows cannot later become complete. Native lifetime checks stay outside the callback lock. Live tiles/table use the latest resolved original interval and show its clock while delivery is pending; a latest lost interval shows unavailable rather than older busy values. Hover/pin selects the exact original pending/completed/lost row. Curve, overview, tiles and contributors share one nonblocking read revision, and late completion invalidates cached geometry. The fixed native interval reservation enters the existing 256 MiB admission budget and settings estimate; Off releases it with the owned trace. No raw event log, second collector or per-CPU trace is added.
    Chart hover gives time, CPU, interval load and sampled parking on separate unwrapped rows, positioned within the viewport at either endpoint. Amber parking ticks sit below the curve at observed samples; adjacent flags never imply continuous parking. Interval load remains visible even when the CPU is parked at the poll. Both graph batches remain cached across pointer movement.
    CPU modals default to the current group's logical CPUs; All CPUs is transient and resets on opening. The explicit System target instead opens all logical CPUs and host processes, requires no saved group and has no group-mask toggle; it reuses the same graph/process presenter and existing telemetry. Process graphs retain a visible outside-current-mask activity count with a Show action even while filtered, and active outside tiles use red borders/text. Unknown data is not a clear result; earlier masks are not recorded. Each tile has a cached mini graph from at most 64 min/max buckets, retaining peaks and missing intervals. Clicking any tile selects its original-sample detailed graph below. Hovering the detailed graph reads one exact all-CPU historical row for tile percentages and sampled parking, marks the grid History with its local timestamp and draws synchronized mini-graph cursors; leaving restores Live without interrupting collection. Stable pointer movement reuses plot geometry and takes one UI pass; a changed layout can request one bounded relayout. Missing/evicted/contended historical data stays unavailable. All these reads use existing RAM history, with no extra native poll or per-CPU trace.
    CPU modals use 80% of usable window height with roughly ten-percent top/bottom margins, retaining compact width and a larger stable contributor viewport. Group CPU windows default to tracked apps on each opening. The transient All system processes chip opts into other observed host processes; its scope is shared by Graphs and Processes. It switches live CPU tiles, cached mini/detail history and contributor rows together. Tracked means the unique PID/creation-token union across all saved groups at the original sample; application graphs keep their own exact rule scope. The existing worker aggregates the already deduplicated tracked instances from the same scheduler sample into one additional preallocated per-CPU history ring. Historical contributor unions deduplicate exact lifetimes, retain original rule membership and distinguish reused PIDs. Missing aggregate data remains unknown and known sparse rows remain partial; scheduler failures are visible in tracked scope. The extra numeric ring enters settings estimates and the existing 256 MiB admission budget. Filter changes invalidate scope caches without creating a collector or native read.
    Below the selected CPU's detailed graph, a compact Process/PID/CPU table sorts contributions by descending residency on that logical CPU. Live uses the latest completed graph sample; hover uses that exact historical row with an explicit History clock. Group tables include observed host processes, not just saved apps; app tables retain the exact rule membership and creation-token instance for that sample. A preallocated sparse numeric ring shares bounded lifetime/name metadata (8192 historical identities) instead of dense PID/CPU/time matrices. Exceptional pool pressure evicts complete old contributor frames; missing/partial attribution is explicit and is never replaced with a current PID roster. Numeric history plus the contributor reservation stays within the existing 256 MiB admission budget. Modal host load and contributor rows share the original scheduler interval; unknown attribution remains partial and is never forced to match the compact current PDH meter. RAM-only cursor reads reuse cached rows; a fixed-height internally scrollable table keeps the plot stationary when historical process counts change. Settings include the contributor reservation and retained metadata in the estimated extra memory.
    A missing tracked sibling keeps the complete app/tracked-union total unknown, while a separately accumulated positive measured subtotal remains visible. The same worker fills bounded preallocated current subtotal vectors; historical quality uses the existing validity byte (unknown/complete/partial), adding no dense history ring. Exact-time tiles and detail percentages mark partial values with *, the selected header identifies Partial, and cached detailed/mini curves use dashes with hover/legend explanations. Complete values are solid, isolated valid samples are dots, and intervals with no positive known evidence or no data remain gaps. Quality survives retention resize and resets with overwritten rows, membership revalidation and Off; host busy-time graphs are unchanged. Settings include the small current subtotal allowance; no new trace or native read is added.
    CPU selection uses a green two-pixel outline, muted green tile fill and matching green name-badge background and accent rail without changing tile geometry; no selection check is painted. CPU names use one shared themed rounded badge with a subtle outline in tiles and the selected detail header. Tile badges occupy 70% of the tile inner width and sit centered at the top; parking and percentage share a centered, font-height bottom row, leaving the name distinct. Parking keeps its own text color; activity outside the mask keeps its red warning alongside selection. The selected state is exposed through the tile's accessible button metadata and keyboard focus ring. The large graph and process table share one outlined card with a short matching header accent, equal eight-pixel insets and a selected-CPU badge; the load column and zero/unavailable messages name that CPU. Process names align left and numeric columns right in fixed cells. All three monitoring tables use fourteen-pixel rows with one-pixel spacing and subtle physical-pixel horizontal separators; their scoped density does not inherit button height. Virtualized scrolling and stable viewport IDs are retained. Static decoration starts no timer, native read or additional sampling.
    A left click in the detailed plot pins its exact original sample, keeping the cursor, all CPU tiles and process contributions at that time while the pointer leaves and the list scrolls. The top mode shows Pinned with its clock; pinned inspection suppresses the pointer tooltip and continues recording. The next left click anywhere in the application unpins once per physical event, preserving the clicked control's ordinary action and resuming hover or Live. Wheel and drag events do not unpin. Expired samples remain unavailable without nearest/current substitution; closing, reopening or disabling resets the transient pin. Selection state adds only bounded scalar UI data and no new collector or history allocation.
    Application CPU tiles show a five-pixel yellow activity dot inside the CPU-name badge when that exact rule has positive measured work anywhere in the currently retained window. Its hover explanation identifies this as retained app work, independent of Live/History/Pinned inspection. The cached overview supplies one reusable boolean per CPU; new work adds the dot, and expiry/source removal/shorter retention removes it. Missing samples do not prove inactivity. Group graphs do not show the app marker. Badge text reserves space for the dot, with identical tile geometry whether it is present or absent; the former right-hand green rail is removed. This static decoration adds no native read, history scan, timer or telemetry ring.
    Application and group CPU modals use one shared header/filter/body/footer layout with the bottom Graphs / Processes segmented switch. Application Processes retains the virtualized current exact-owner PID list including idle and unavailable tracked instances, whole-machine CPU time and measured active logical CPUs; complete idle, partial and unavailable mapping remain distinct. Group Processes reuses the same table renderer and aggregates the latest finalized original contributor interval across the displayed CPUs, preserving exact PID/lifetime identities and deduplicating shared rule/CPU contributions. Group percentages express the share of displayed CPU capacity, with an explicit scope line and column explanation; an idle process cannot be assigned an observed CPU. Lost/missing intervals stay unavailable and incomplete measured rows are marked partial. The derived Arc-backed group table is cached by original time/history revision/CPU mask/process scope, released on close/Off and included in the memory estimate; no extra collector, native read or history ring is added. The same 1 Hz worker retains each existing GetProcessTimes result before app aggregation and copies positive per-PID scheduler values, sharing immutable current rows with UI through Arc. Names are cached by exact PID/token after verified path/token inspection on the worker, independently of trace admission. Membership revalidation invalidates pending row values; source additions/removals and Off follow existing lifecycle. Table mode performs no native read or full-history copy; graph inspection/pinning is separate. The memory estimate includes current row storage without adding a historical per-PID matrix or collector. The Process monitor roster and CPU telemetry derive ownership from RunningApps but have independent refresh/validation and display filters, so a tracked PID roster is not a selected-CPU contributor list.
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
  - the Process monitor modal's retained tracked-process presentation rows, one-second refresh admission and unavailable-data status; ordinary GUI passes borrow the cached rows
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
- The application-rule editor uses full-width labeled name/path fields rather than a narrow grid. Browse shares the path label row; overflowing paths also show a complete selectable, soft-wrapped value below the single-line editor, with word-boundary-independent wrapping and no path mutation. Priority/startup, arguments and process tracking remain grouped in the existing scrollable card. Add controls share section headings, descendant handling stays alongside explicit process-name matches, and shortcut status wraps separately. Saved identity, target restrictions, same-frame dirty shortcut gating and save/delete/cancel dispatch remain unchanged.
- App settings offers a persisted `start_minimized` preference. The Windows entrypoint loads the canonical storage once before creating the root viewport, requests initially hidden/inactive/no-taskbar when enabled and passes that same storage into `AppState`. The shell starts hidden only with a usable tray; tray failure or blocked cold shortcut forwarding shows the error window. Explicit viewport visibility replaces offscreen positioning; restoring preserves geometry and ignores stale minimized input until Windows acknowledges restoration. Each taskbar transition updates both Winit's retained `skip_taskbar` policy and the existing native styles, including visible fallback, so `TaskbarCreated` cannot reapply an old hidden policy. A one-line local eframe 0.36.2 patch prevents its first-paint automatic show from overriding initially hidden roots. The preference defaults to false, remains readable in schema v10/v11 and affects only the next launch. No Windows login registration or `--start-in-tray` flag is added. See `docs/hidden-startup.md` for native evidence and remaining release smoke.
- App settings keeps monitoring, history and startup explanations in bounded hover tooltips. One `Extra memory: ~N MiB` row estimates monitoring's additional cost using calculated history, the configured maximum trace buffers and runtime/tracking allowances; its breakdown is on hover. This estimate follows retention, saved apps and observed tracking changes and is not measured whole-app memory. Preference and collector errors remain visible in the menu. Its lifetime remains independent of the nested history selector.
- Tray hiding also normalizes only the calling GUI thread's exact layered/nonactivating `Winit Thread Event Target` popup to a restored zero-sized state. The helper stays visible for Winit's paint-event delivery; other windows are not hidden or restored. Native regression coverage reproduces a leftover minimized helper rectangle; the Task Scheduler login scenario remains a manual smoke check.
- The header Process monitor button opens the shared service-modal layout with Enable/status and Processes / Logs tabs. Only Processes shows the tracked count/updating state; Logs does not imply a fresh roster. Empty lists have a compact explanation without table headings, rendered outside zero-row virtualization. Populated Processes retains a window-bounded virtualized viewport. Process, PID and Group use the same 14-pixel rows, one-pixel gaps and subtle physical-pixel separators as CPU tables; long name/group cells retain horizontal drag/Shift-wheel without consuming normal vertical scrolling. Name/PID/Group headers sort in both directions. Refresh attempts occur at most once per second only while Processes is open; busy reads retain previous rows, sorting and scroll. Unchanged completed rows keep their allocation; completed empty snapshots clear them. Each refresh still validates exact token-bearing logical rule owners and image paths; this roster does not enumerate host processes or imply activity on a selected CPU. Logs selects at most 100 retained typed monitor events from the existing bounded application log, without a second ring or collector.
- `shell::presenters::process_table` owns compact fixed cells, accessible clickable sort headers and cached index order shared by current app processes, historical contributors and the tracked roster. Name sorting ignores case, PID/load sorting is numeric, unknown/nonfinite load remains last in both directions, and exact PID/lifetime ties are deterministic. Reordering occurs only on a changed immutable snapshot/source revision or sort choice; ordinary paint/hover reuses Arc-backed indices. Late original-time completion invalidates contributor order with the same source revision; sampling and history layouts are unchanged.
- CPU tiles use four-pixel horizontal/two-pixel vertical insets, zero internal vertical spacing and eight/five-pixel horizontal/vertical grid gaps. Their mini graph meets the centered parking/percentage footer without inherited button-height padding; narrow footers clip parking text before the load can overlap. Application modals identify their application scope and cached configured process basenames plus descendant policy; group headers identify CPU-group scope. All CPUs and the group-only All system processes opt-in stay at the left of one row, with the clipped Live/History/Pinned mode and clock aligned to its right edge; active chips use a contrasting success-color fill/border and inverted text. Their explanations and existing transient filtering semantics are retained. Shared modal construction uses one static 185/255 black scrim; no blur/render target or idle repaint is added.
- The light palette uses neutral soft surfaces (#E7E9E8 background, #F0F1EE cards, #DFE3E1 inset, #E4E7E4 rows) with adjusted neutral text/borders. Dark and semantic success/warning/danger colors, font definitions and rasterization options are unchanged.
Windows runtime flow:
1. Entry point parses startup arguments into a narrow startup intent; normal GUI startup remains the default, while `--run-rule <group-id> <rule-id>` is accepted as a saved-rule startup intent.
2. Windows entry point prepares the local shortcut-forwarding endpoint before creating `tokio` or `eframe`:
   - normal GUI startup claims the primary guard; a repeated launch forwards ShowWindow and exits before loading state, autorun, Tokio, GUI or monitors. The endpoint remains scoped to the Windows user, session and executable path, keeping separate portable/test copies independent.
   - ShowWindow uses the existing tray visibility/taskbar/unminimize and bounded foreground activation path plus informational taskbar attention. The launcher grants foreground permission only to the actual connected same-session pipe-server PID before sending this command; saved-rule commands do not request activation. Windows retains the final focus decision; no topmost state, geometry change, input attachment, listener or idle poll is added.
   - endpoint/guard/runtime initialization and forwarding failures fail closed. Failure to install the primary command server aborts app creation before startup intent/autorun dispatch.
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
- concrete registration supplies fifteen managed adapters (including the separate manual core-parking adapter, the background-session preview, five compact Windows preference/display adapters and the retained optional process timer-policy experiment) and ten read-only adapters without native work during construction
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
- System Tweaks uses one permanently visible horizontal segmented switch with three sections: System, GPU / network, and Check PC. Full section descriptions are available on hover. System opens by default and contains six game settings and two comfort settings with direct Apply/Undo, visible scope/current value and explanations in help; the manual Memory integrity guide follows the game settings before For comfort. Only the selected content scrolls; section selection is transient and starts no native operation. GPU preference targets one qualified saved EXE; hardware guides remain manual and Check PC requires an explicit read-only request. Global errors, Undo and display confirmation remain reachable across sections. There are no visible profiles, triggers or duplicated process affinity/priority controls. Each manual tweak reuses one hidden existing-schema profile/target intent. Old profiles, associations and recovery data remain retained internally; only old module settings with actual owned fields appear in a conditional block at the bottom of System, without profile names or saved activation flags. Each old setting has its own Undo, which disables only that saved module and uses existing conditional restoration. Unresolved ownership remains visible for retry even after withdrawal; no empty restoration block or separate Restore section is shown. Already-suitable values create no owner or Undo baseline. Cards distinguish exact stored preferences from effective/game results, restart requirements, drift, failure and restoration.
- Display refresh changes only a tested compatible nominal Hz at the current geometry/color/scan mode and requires its exact original mode to be in the tested inventory before acquisition. It is a temporary user-session change with a 15-second confirmation; pending intent is durably distinct, and expiry, closing or startup attempts exact restoration. Failed withdrawal retains evidence and reports recovery required. Confirmed settings are not written as persistent boot display modes. An unowned display card rediscovers the current primary screen; active or unresolved ownership retains its exact original target.
- New GPU preference activation qualifies an existing saved EXE. Owned registry reads and conditional restoration retain the exact user/path even after EXE removal; UNC qualification preserves the network root. This does not bypass user/session or expected-value checks.
- The Windows viewport has a fixed width of 400 logical pixels and permits vertical resizing from 600 to 1000; the tuning page is bounded to 400. Before first paint, the entrypoint retains these min/max limits as Winit logical sizes (including initial egui zoom), replacing egui-winit's startup physical-pixel limits. Winit owns per-monitor DPI/position/min-max callbacks; existing egui DPI input and WGPU resize paths update rendering, with no application DPI subclass or repeated size forcing. Tweak cards, including manual guides, fill the available width and reserve a fixed right action column; Undo replaces Apply in the same slot. The remaining application layout and classic saved-EXE workflow remain shared.
- Each CPU topology section has a separate Select all / Clear all action above its core grid. Its 17-logical-pixel height is half the 34-pixel core tile height; bulk selection and clearing remain limited to that section, including free cores, with the existing keyboard/focus semantics.
- AC power now exposes one direct Apply/Undo for the Windows AC performance preference. CPU frequency sliders, high-clock and parking controls within this power-mode adapter are retired; configured requests for those options are rejected. Core parking is offered separately with its own manual adapter and exact Undo. New applies contain no CPU policy or DC fields, and observation no longer queries CPU topology. Legacy saved CPU options, exact base/Performance-overlay bindings, wider values and nullable originals remain intact for recovery and Undo; the card identifies previous CPU settings. Existing owned changes must be undone before applying the simpler preset. Verification describes the stored Windows power preference, without asserting physical clocks or FPS.
- CPU AC overrides retain absent versus explicit DWORD originals. Scoped caller-token SeRestorePrivilege is checked before creating an override and restored immediately; exact absence Undo reopens a retained setting object with RegCreateKeyExW and an empty subkey, deletes only ACSettingIndex, and never changes ACLs or deletes setting containers. A module-owned confirm_applied hook repeats guarded runtime refresh for matching recovered bytes or an already-restored baseline before retiring evidence. Other adapters use its no-op default. Refresh never selects a different captured plan. Real Windows 11 limit 4500 MHz/high-clock-off and high-clock-on round trips passed on 2026-10-15, including disk reload, exact absence Undo, unchanged active plan/DC preference and parking; this does not prove measured clocks or FPS on other hardware.
- old canonical continuity/configuration-receipt/handle-transfer laboratory source was removed with recoverable local snapshots; existing ProgramData journal/staging/previous evidence is detected read-only and never discarded or overwritten
- native handles, typed user/process reads and conditional writes, and read-only display/audio/PnP inventories live in `libs/os_api`; application code owns configuration and lifecycle
- tests and contribution rules are in `src/app/features/system_tweaks/README.md`; product scope is `docs/modular-gaming-features-plan.md`
- System Tweaks lives entirely in `src/app/features/system_tweaks/`; each managed tweak owns a folder under `tweaks/<name>/mod.rs`. `platforms/windows.rs` composes the Windows catalog; `platforms/linux.rs` is an inactive extension point. Each adapter declares its Windows 10/11 minimum build. Cached read-only real Windows build detection gates observation and activation; unsupported cards remain grey with a reason. Recovery/Undo is retained independently of activation eligibility. Serialized `tuning` data, stable feature IDs and recovery targets remain compatible. Windows 10 compatibility is declarative and unit-tested; native round-trip evidence remains from Windows 11.
- Operation feedback belongs to the current request: a new request clears the previous transient error/restore result while unresolved controller evidence remains visible. Autonomous retries clear their own prior errors; confirmed withdrawal forgets rejected attempts only without field/record/pending evidence. Restoration preserves the strongest restart requirement and distinguishes partial results. Disabled platforms never reserve the classic affinity/priority fields from retained tuning associations.

Background CPU placement joins bounded x64 GLPI physical-core records with CPU Set IDs, groups, efficiency classes and group-relative cache/NUMA metadata. The optional efficient/manual pool is stable per session and retains whole SMT cores. Placement uses the canonical process-default CPU Sets field, exact original selection (including empty), complete topology comparison and conditional native readback. Existing process hard affinity and thread-selected/individual thread restrictions can refuse only placement; priority/EcoQoS remain separate. Native multi-group process placement and physical cores spanning groups remain unqualified and are visibly skipped, while topology/group IDs remain representable. Pool records are bounded to 129 and retained evidence to the existing per-field/shared budgets; oversized placement evidence drops that optional field rather than blocking priority/QoS. Native owned-child tests prove mechanism round trips only, not actual role admission.

## Concurrency model
- GUI runs on the main thread
- System Tweaks session observation/reconciliation runs only for enabled non-manual rule associations or retained Session ownership. The worker caches that demand across idle queue checks and reconciles at most once per second or immediately after an accepted request; manual-only/undone configuration does not start process observations or repeated saved-state verification. Native mutation, manual inspection, display expiry and shutdown retain their independent verification paths.
- Positively expired compact session ownership also withdraws its saved manual intent. An unowned non-EXE card discovers the current native target, including after interruption between recovery and intent publication. At the 64-profile admission limit, new compact targets may reclaim only disabled hidden profiles without associations, controller records, pending/error evidence or display confirmation; legacy profiles and monotonic IDs remain intact. Unresolved Undo is never capacity-reclaimed.
- Classic save methods return queue acceptance in all builds. Tests that need completion explicitly call a test helper; gated tests distinguish pending, successful and failed persistence. The retired transient profile editor is absent; saved legacy configurations and conditional Undo remain supported.
- background tasks use `tokio`
- the tuning application runtime uses one dedicated standard thread and bounded request queue for saved candidates, owned-field operations and execution observations. It publishes immutable snapshots; shell polling adopts them while hidden and during bounded shutdown. Native reads never hold GUI-facing snapshot/admission locks.
- Full System Tweaks card/capability refresh runs only while its page is visible and uncovered. The worker continues owned-field inspection, execution/session reconciliation, display-confirmation expiry and shutdown recovery while hidden. Reopening requests a fresh card snapshot on the same worker. Compact cards use a combined observation/readiness seam; display refresh reuses one tested mode inventory per card refresh. Apply/Undo still perform their independent native validation.
- CPU graph modals suppress rendering of the underlying active page, keeping the header/footer and ordinary eframe scroll distance and input. During modal wheel smoothing or dragging, a 60 Hz frame budget defers painting before consuming native input, using the existing event-loop deadline without sleeping the GUI thread. Buttons, keys, text, viewport changes and close bypass it immediately. No input is discarded, no new thread or idle timer is added, and actual cadence depends on native presentation. Uncovered pages and non-scrolling idle/hover retain reactive rendering. Each retained original CPU interval additionally stores a bounded one-byte gap category (baseline, numeric contribution capacity, stream/schema/order failure, missing frame or other known failure). Native frame causes propagate to MCP quality and opt-in render logs; resize/overwrite/Off preserve the original-time lifecycle. A category never turns an unknown interval into zero or asserts a more precise cause than was observed.
- Windows shared-font preparation feeds egui's own caches in at most eight-character portions once per physical UI frame. The fixed font/character set covers four subpixel positions and restarts at changed pixel scale or actual text options; temporary eight-millisecond repaint requests end after preparation completes. It paints no hidden screen, adds no worker or second text cache, and does not pre-render arbitrary Unicode process names. CPU tiles reuse one measured size qualified by width, scale, font height and spacing; fully clipped tiles reserve that geometry without laying out their labels. Exact current PID display strings follow existing one-second snapshots, invalidation and memory accounting; contributor PID/load strings follow the existing exact-time view cache. Table virtualization, cursor selection and telemetry frequency are unchanged.
- The fixed native numeric interval reservation allows 128 positive contribution cells per logical CPU (at least 256), retaining all 16 pending/frame buffers. This startup/churn headroom is reserved once on enable, included in settings estimates and the same 256 MiB admission budget. Exceeding it remains an explicitly categorized gap; buffers do not grow in the event callback.
- tray callbacks only enqueue typed commands and wake egui; `tray_rx` is owned and drained by `shell::App` on the GUI thread, which exclusively owns `HWND` operations and the terminal Quit transition
- Windows left-double-click Restore arms on the second press and queues only on its completing left release, when the gesture timestamp is captured. Single clicks, other buttons and a later new press cannot consume an unfinished double click. The tiny atomic latch adds no timer or worker; the menu path is unchanged. Nonshipping debug MCP builds can print bounded per-gesture native activation outcomes for acceptance.
- Windows tray Restore shows and unminimizes through the retained Winit window on the GUI thread before painting another frame, then immediately checks native readiness/foreground ownership through `os_api`. It bypasses cached Winit focus and leaves foreground permission to Windows; raw last-input timestamps are not an activation veto because ordinary pointer motion changes them. At most two attempts occur within300ms of the gesture; success, invalid ownership, hide, close or expiry ends the session. It changes no geometry, topmost state or input attachment and adds no idle worker/timer.
- Windows local shortcut-forwarding requests flow through a shell-owned named-pipe server thread into `shell::App`, with per-request reply channels; request enqueue wakes the `egui` context for prompt draining
- `AppForwardingRuntime` clears the GUI wake callback and requests server shutdown during drop. It joins promptly when the worker reaches a terminal I/O completion; otherwise the detached reaper retains the pipe listener, in-flight `OVERLAPPED` buffers, and primary guard until it can safely release them. A replacement process therefore cannot claim the endpoint while that worker still owns it.
- monitor notifications flow through typed `ShellEvent` messages in `monitor_rx` owned by `RuntimeRegistry`; runtime writer guards are released before emitting notifications
- Automatic native process-setting failures use bounded important ProcessMonitor entries in both log views. The existing correction task suppresses identical exact-rule/PID/creation failures for60seconds, retires recovered/withdrawn failures and retains at most256 bounded messages with a rate-limited overflow summary; read-only observers and unresolved owner choices do not produce correction errors. No additional worker or native query is introduced.
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
- current persisted schema version: `14`
- Optional `cpu_load_monitoring_enabled` defaults to true, is retained in ordinary state saves and can stop all Windows CPU telemetry without changing `process_monitoring_enabled`. App settings explains monitoring overhead and stores `cpu_load_history_seconds`: 30, 300 or 1800 seconds, default 300. Histories are memory-only numeric rings, reserved for every saved rule when monitoring starts and bounded to the selected elapsed duration plus one endpoint. Timeline and parking storage are shared, and recording overwrites preallocated cells. Only the visible CPU series is copied into a reusable UI cache; published current-value snapshots do not clone history. Adding a rule reserves its own ring with no pre-addition samples; deleting it releases that ring, and moving a logical rule preserves its history. Existing histories survive process restarts. Retention changes preserve available points without restarting ETW or increasing the 1 Hz cadence; increasing retention cannot recover evicted points. Disable releases all history. Calculated history size plus estimated native buffers/runtime/tracking tables is shown in settings and follows saved-rule, retention and observed membership changes. History admission is capped at 256 MiB with checked, fallible reservation; an oversized request reports an error without changing affinity/priority protection. No CPU-load data or trace files are persisted. Linux retains the preferences but starts no collector. Real-time ETW sessions are stopped on normal closure; an abrupt process termination can leave an OS-owned session alive, so this is not a crash-cleanup guarantee.
- On 2026-10-15 a real elevated Windows 11 CPU-load probe passed: 20 logical-CPU PDH samples, exact-process aggregate and per-CPU scheduler residency during a five-second controlled load, wrong-token refusal, prompt own-session shutdown and an independent logger remaining active. The observed 28-byte CSwitch v5 event uses a source-confirmed bounded ID prefix when local TDH metadata is unavailable; unknown versions still require TDH. This proves native sampling and normal cleanup on that host, not measured game/FPS overhead or universal hardware compatibility.
- On 2026-10-15 the optimized continuous-history probe passed on the 20-logical-CPU Windows 11 host: ten saved-rule histories shared one exact controlled PID with one arithmetic/eight sleeping threads, 1800-second capacity reserved1.951MiB,25seconds recorded before opening. Graphs closed:0.0195% whole-machine CPU (0.3906% one-CPU equivalent),+8.012MiB private/+8.539MiB working set. Addition/deletion, sibling retention, disable ring release and worker stop passed; the native own/foreign-session probe also passed. Details and measurement limits are in docs/release-smoke-matrix.md; these are not universal FPS, GUI/GPU or allocation-free lifecycle claims.
- schema `v5` and older formats are dual-read and normalized in memory without eager rewrite on load
- schema `v6` and older path-target app rules receive an in-memory one-time compatibility backfill that adds the primary executable filename to `additional_processes` when no normalized equivalent already exists
- schema `v7` treats an empty `additional_processes` list as intentional user state and does not re-add the primary executable filename on load
- schema `v9` stores `windows_event_log_diagnostics_enabled`; the pre-release schema-v8 disclosure field is ignored, and v8 and older files are effective enabled without an eager rewrite
- schema `v10` stores each rule's `manage_descendants` policy. Version-aware loading materializes missing values in pre-v10 state as the previous behavior (`true`); missing values in v10 and later, schema-less rule inputs, and new rules use the safer `false` default.
- The real state-file loader accepts retained tuning in both v11 and the current supported schema; a current-format save/reload cannot become read-only merely because tuning is present. Older-schema tuning, malformed/oversized evidence and future state versions retain their refusal contract. File imports skip exact same-group runtime-key duplicates while Steam client and game targets remain distinct. Failed file-import completion preserves the input behind explicit Retry/Dismiss, emits one diagnostic with its refusal kind, and cannot requeue itself on each render; a fresh drop/selection replaces that retry session.
- schema `v13` adds optional raw-preserving built-in helper provenance. Old unbound rules retain their lists/launch settings. Valid unknown bindings remain inert; malformed/older-schema evidence cannot silently become global names. Loading alone does not seed defaults or rewrite the file; explicit saves use the existing atomic writer and retain tuning/unknown data.
- schema `v12` adds explicit Steam-game launch targets; v11 and older saved rules retain their existing launch/tracking meaning until explicit conversion. Older binaries cannot consume new Steam targets; save upgrades the state version without changing existing groups.
- schema `v11` adds optional bounded tuning configuration. v10 and older inputs without it remain Off and are not rewritten on load. An unexpected tuning extension in an older state blocks an upgrade that could reinterpret it as activation. Unknown v11 top-level fields and tuning structural versions/fields preserve their exact raw values through unrelated saves
- state reads consume at most 16 MiB plus one detection byte. Oversized input remains on disk with a finite save refusal; bounded unsupported/future or malformed explicit version headers and duplicate top-level names retain raw evidence and a private save block. Tuning evidence is guarded before unrelated base-schema decoding can enter recovery. Clone and explicit schema preparation cannot clear that block. Serialization is bounded to the same 16 MiB before backup, directory creation, staging or publication. Ordinary syntactically invalid JSON below the bound retains the existing copy-before-default recovery path. Failure to secure that recovery backup blocks later saves, including cloned/schema-prepared state, until storage is repaired and loading is retried; the original is never replaced by writable recovery defaults
- the upgrade from pre-`v6` data or `v6` data to the current schema happens only on an explicit save path
- before the first current-schema save after loading pre-`v6` state, persistence creates an additional `state.json.pre-v6`, `state.json.pre-v6-1`, and so on backup series
- loading `v6`, `v7`, `v8`, `v9`, or `v10` for upgrade to `v11` does not create a `pre-v6` backup
- after the first current-schema save, downgrade to an older binary that only understands earlier state is unsupported
- backup rotation uses `state.json.old`, `state.json.old1`, `state.json.old2`, and so on
- persistence loading is split into `state_path`, `storage_io`, `migrations`, and `schema_refresh`; saves stage and sync a same-directory temporary file before an atomic replacement on Windows or an atomic rename plus directory sync on Linux. Windows does not claim parent-directory durability across power loss; recovery and migration backups are copy-and-sync operations that preserve their source before publishing.

Key entities:
- `CoreGroup` - CPU core group plus assigned apps
- `AppToRun` - application launch configuration with `Path`, `Installed` or `Steam` launch targets and optional built-in helper provenance
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
- Windows local named-pipe and mutex transport for single-instance restoration and saved-rule shortcut forwarding
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
- `AppsFolder + Start Menu shortcuts + App Paths` installed app discovery and AUMID activation. Package lookup validates AUMID syntax with Windows and passes the family as child-local environment data to a constant PowerShell script; `.url` decoding reads at most64KiB plus one detection byte before decoding
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
- `eframe` / `egui` - desktop GUI; Windows uses WGPU/DX12 with `MemoryUsage`, while Linux retains Glow. Root Cargo patches select the checked-in eframe/epaint 0.36.2 copies for initial visibility, opt-in pre-input root-frame deferral and equivalent-default hinting coordinates (`vendor/eframe/LOCAL-PATCH.md`, `vendor/epaint/LOCAL-PATCH.md`). The latter uses the existing sleeping event-loop deadline, retains pending input and bypasses native lifecycle events; default clients and the Linux entrypoint retain upstream repaint behavior.
- `wgpu` - Windows-only DX12 backend and typed device memory hints; validation layers are disabled for ordinary operation
- `tokio` - background runtime with `rt-multi-thread`, `time`, and `sync`; worker count remains the runtime default
- `rmcp` / `axum` / `tokio-util` / `subtle` - Windows production MCP protocol, localhost HTTP admission, cancellation and constant-time bearer comparison; they do not introduce another executable or CPU collector
- `windows` - Win32 bindings for shell integration, process/runtime operations, local IPC, security descriptors, and manifest/resource-adjacent Windows APIs
- `tray-icon` - Windows tray integration
- `rfd` - file dialogs
- `serde` / `serde_json` - persisted JSON schema
- `regex` - CPU preset matching and related helpers
- `once_cell` - lazy initialization
- `num_cpus` - logical thread-count detection
- `image` - Windows window/taskbar/tray ICO decoding and explicit PNG graph export; only `ico` and `png` features are selected. Startup uses the same bundled ICO for native icons and the EXE resource, moving each decoded RGBA buffer into its owner; PNG work runs only after a user save request.
- `raw-window-handle` - direct Windows native-window handle access
- `winit` - the same locked GUI dependency, also explicitly selected on Windows for its existing taskbar policy API and test-thread event-loop extension. The local 0.30.13 Cargo patch accepts Windows 11's `WM_DPICHANGED` rectangle directly to avoid monitor-nudge feedback and runaway growth; known pre-22000 builds retain the old adjustment. No new event loop or input layer is added.
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

The Windows CI verification job has a 60-minute budget, matching the stable Windows release build job, so cold optimized application/test builds and final cache compression can finish the existing gates. Linux CI retains its 30-minute budget.

The root package alone overrides the test profile to opt-level 1 and limited debug information. Dependency/dev/release profiles, debug assertions and arithmetic behavior remain unchanged. `scripts/measure-test-build.ps1` records alternating root-only rebuild/test timings with cached dependencies; it does not measure a cold full CI job. `docs/testing.md` records the deterministic/live coverage split and consolidation limits.

`scripts/test-windows-live-mcp.ps1` runs synthetic built-app acceptance against a separate nonshipping debug-MCP/diagnostics executable: real UI input, production HTTP configuration/receipts/restart, schema-upgrade failure/retry, extra/shared CPU/RAM unions, history/CSV and Steam warning/selection. It requires elevation for protected diagnostic authority and ETW, owns fresh sidecars/keys/processes and treats forced cleanup as failure. Ordinary launch/native readback/Focus is an explicit interactive opt-in; shipping bootstrap and real vendor/VM smoke remain distinct. Windows CI reuses the crash probe's diagnostic build via `-WithDebugMcp`, then runs the default live pack and uploads only sanitized result JSON for seven days. Stable release gates remain unchanged.

The same runner's optional `-CaptureReadme` mode uses the fixed eight-scene contract and owned measured workloads for documentation only. It is separate from the default acceptance pack. Windows and Linux CI check the committed PNG set, checksums, composition metadata and cleanup provenance without launching a capture or updating assets.

Local verification commands:
- `cargo test --manifest-path libs/os_api/Cargo.toml`
- `cargo test --features windows --bin cpu-affinity-tool`
- Windows CI additionally runs the local Winit library's `dpi_` rectangle-policy regressions with `rwh_06` and no default features, using a temporary standalone lock seeded from the root lock.
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
- `.github/workflows/ci.yml` cancels superseded runs per branch or PR, restores Rust build cache, keeps the Windows release-path checks on `windows-latest`, tests local eframe frame-deferral predicates/deadlines and epaint hinting regressions with temporary locks seeded from the root lock, runs feature-gated real-binary crash probes and isolated forced-process tuning recovery probes, reproduces the stable line-table release build, verifies the built EXE/PDB identity and manifest resource, and verifies the Linux beta binary on `ubuntu-24.04`
- tests are part of the committed CI contract for `ci.yml`; both jobs require committed lockfiles to remain unchanged after compilation/tests, and the Windows release build reports EXE/PDB byte sizes. The Windows job also runs the MCP launcher's filesystem isolation self-test without launching an app or requesting elevation.
- the Windows CI job validates the feature-gated Windows binary path explicitly with `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --features windows --bin cpu-affinity-tool`, `scripts/build-windows-release.ps1`, `scripts/test-windows-pdb-verifier.ps1`, and `scripts/assert-windows-release-manifest.ps1` against the built release EXE/PDB pair

Current release facts:
- stable GitHub Release workflow reacts to pushed tags matching `v*`
- the stable release workflow validates that the tag matches `vX.Y.Z`, that `Cargo.toml` version matches `X.Y.Z`, and that `changelogs/vX.Y.Z.txt` exists before building
- the stable Windows build job restores Rust cache, runs `cargo fmt --all -- --check`, `cargo clippy --features windows --bin cpu-affinity-tool -- -D warnings`, `cargo test --manifest-path libs/os_api/Cargo.toml`, `cargo test --features windows --bin cpu-affinity-tool`, runs the same local eframe/epaint/Winit regressions as Windows CI plus `scripts/test-windows-crash-reports.ps1` and `scripts/test-windows-tuning-recovery.ps1`, builds `cpu-affinity-tool.exe` plus `cpu_affinity_tool.pdb` with `scripts/build-windows-release.ps1`, verifies the EXE/PDB basename, GUID, and age with `scripts/assert-windows-pdb-matches.ps1`, and then verifies the built exe manifest resource with `scripts/assert-windows-release-manifest.ps1` in the same runner before upload
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
- version truth is split across Git tag, `Cargo.toml`, its local package entry in `Cargo.lock`, and `changelogs/`
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
uses request-specific captures on the settled reply frame. Mutations/input return
only an applied/frame acknowledgement; `cpu_state` is a compact snapshot unless
`include_series=true` explicitly requests the selected numeric series. Table reads
prepare only the requested bounded page; quality does not materialize a series.
Font preparation covers four subpixel glyph variants in bounded per-frame chunks
and invalidates on actual font TextOptions or pixel-scale changes. It reuses egui
caches and requests no repaint once settled. Its optional profiler stage is
`font_preparation`; ordinary release compilation removes that instrumentation.
The CPU-modal scroll budget defers root paints at 60 Hz before native input is
consumed, preserving queued wheel data and bypassing discrete input; it never
sleeps the GUI thread. No additional worker or idle repaint timer is introduced.
The debug feature
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
- local package version in `Cargo.lock`
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
