## 2026-10-11 compact System Tweaks revision

The accepted manual interface stays English and is named **System Tweaks**. Each native tweak owns `src/app/features/system_tweaks/tweaks/<name>/mod.rs`; the page, persistence, worker and shared controller stay inside `system_tweaks`. Adapter-owned Windows 10/11 requirements grey unavailable cards and block observation/activation before native work. Linux has an empty composition seam for a possible later implementation. Stable persisted feature IDs, `tuning` schema-v11 data and conditional Undo remain unchanged. Windows 10 support is a tested compatibility map; actual native validation so far is Windows 11 only.

# Modular gaming features

The product consists of four stages: UI, a small module registry/controller, real modules, and persistence with safe lifecycle/rollback. Stage identity and acceptance status live in the local `.codex/ROADMAP.md`, not this document. The earlier general transaction/continuity lab plan is superseded.

Modules are currently Windows-only. Linux hides the Modules page and profile associations, registers no modules and runs no module observations, recovery or native actions. Its existing background queue only saves ordinary application state/rules, retaining raw tuning data unchanged. System Tweaks now has separate Windows catalog composition and an inactive Linux extension point; Linux native tweaks remain out of scope.

Native acceptance evidence is from Windows 11. Linux native tweaks and local Linux verification are outside this implementation's acceptance; existing repository CI and beta release definitions remain in place. Windows 10 support is declared and unit-tested, with native Windows 10 validation still open.

## User flow

System Tweaks is a compact manual page in a fixed-width 400 logical-pixel window: choose an individual setting and use Apply or Undo. A horizontal segmented switch selects System, GPU / network, Check PC or Restore; only its selected content scrolls. Scope and current value stay visible; help opens on hover or click. No profile or process trigger is required. GPU preference alone selects an existing saved EXE. CPU affinity and priority remain in Overview. Existing schema-v11 profiles/controller/recovery supply hidden manual intents; previous profiles and associations are retained in Restore. Section selection does not apply a tweak or read PC diagnostics.

Five game settings cover AC performance preference, Game Mode, background clips, compatible windowed-game optimization and main-screen nominal Hz. Two comfort settings cover accessibility activation popups and mouse acceleration. Hardware guidance lives in GPU / network; six cached read-only PC checks run only after the explicit Check PC action. Guidance is not an automatic BIOS/driver/network modification or a measured performance result.

Saved intent, observed state, verified application, unsupported/denied settings, external changes, failure and restoration are separate. A requested value is not an observed result. Module operations and state saves run off the UI thread. Unknown configuration remains visible and read-only.

## Module set

| Module | Initial behavior |
| --- | --- |
| `power_plan` | Select an existing Windows power scheme; restore the original GUID. |
| `power_mode` | Compact preset changes only AC performance preference, preserving battery preference; effective mode can differ. Legacy AC/DC configurations remain readable. |
| `mouse_acceleration` | Own only acceleration fields, preserve pointer speed. |
| `accessibility_hotkeys` | Manage activation shortcuts while preserving active accessibility functions and unrelated fields. |
| `process_qos` | Manage the documented execution-speed policy for an exact process instance. |
| `cpu_sets` | Select actual CPU Set IDs/groups; coordinate with the existing affinity writer. |
| `process_priority` | Select a supported non-Realtime class; coordinate with the existing priority writer. |
| `process_timer_policy` | Separate opt-in experiment using the documented per-process timer-policy bit; no global timer forcing. |
| `measurements` | Import bounded normalized frame-time CSV, summarize pacing and compare compatible A/B captures. |
| `display_audit` | Observe current resolution, nominal refresh and display adapters. |
| `latency_advisor` | Current observations plus game/driver queue, cap/VRR and measurement guidance. |
| `audio_diagnostics` | Observe endpoints and real audio session clients, with mapping limitations explicit. |
| `irq_diagnostics` | Observe present devices, processor groups and stored interrupt preferences. |
| `usb_topology` | Show actual parent/controller relationships and shared controller scope. |
| `game_mode` | Conditional stored user preference write, exact absence/baseline undo; restart game. |
| `background_capture` | Only the stored Record what happened preference, preserving other recording controls. |
| `windowed_optimizations` | Own only the stored windowed-optimization field; preserve other graphics fields and Auto HDR dependency. |
| `gpu_preference` | Own only GPU preference for one existing saved EXE; preference is not proof of renderer selection. |
| `display_refresh` | Tested compatible temporary main-screen Hz, 15-second keep/undo and exact recovery; geometry/scan/color fields retained. |

Core parking, HAGS, IRQ/MSI mutation, GPU vendor writers and audio-engine pinning remain conditional/advanced research. HPET/BCD changes, global timer hacks and stripping security/services/updates are excluded. Read-only diagnostics do not establish effective ISR/DPC placement or click-to-photon latency. See [the research and disposition table](windows11-gaming-module-research.md) for evidence and scope.

The power-mode adapter uses the documented Windows 11 user-configured AC/DC APIs. Other system signals can override the effective mode. [Microsoft API contract](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powersetuserconfiguredacpowermode).

## Simple implementation

One `id -> Module` registry provides metadata, typed configuration/editor, observations and conditional field operations. Only the registration boundary imports concrete modules. Modules use `libs/os_api`; OS code does not import egui or application configuration.

One controller owns first baseline/current/attempted state and owners for each exact target/field. One background application worker saves candidates, consumes existing execution/foreground tracking, applies requests and publishes immutable UI snapshots. There is no second process monitor or general transaction platform.

CPU Sets/priority use a small field-selection handoff with existing launch/monitor writers. A relevant setting must be captured before its first writer; an already classic-managed instance with no reliable original requires an honest restart indication. Classic rules retain their existing behavior.

## Persistence and restoration

Save configuration before activation. Before a persistent mutation, durably publish a compact record containing the original, current/attempted value, exact target, module/version and owners. Equal requests keep the first original; conflicting requests are refused without writes. Persist zero-write ownership transitions too.

Restore only owned fields and only when their observed values match the app-written value. Preserve unrelated current fields and original absence/raw types. Partial write/save/restore failure, external drift or Unknown retains evidence and a visible error. Delete recovery data only after confirmed restoration or confirmed absence of a write. On restart compare unfinished records with native state before admission; do not recapture their baseline.

Process-owner expiry requires positive completion of the same retained instance. Missing membership or observation failure is Unknown. Foreground loss releases only the foreground owner. A bounded Quit timeout retains the same working context and cannot report successful cleanup.

The application opens native admission after verified saved configuration and disk-recovery initialization. Failed source/lease/legacy checks and unresolved evidence keep it closed. Manual settings persist across application closure until explicitly disabled; Quit releases Session owners with verified authority. All eight adapters passed real native apply/read/disable/original verification on the Windows host on 2026-10-08; see [validation evidence and limits](tuning-native-probe.md). Tests of fixtures prove controller behavior, not native module readiness.

## Delivery flow

Each of the four stages has one whole plan review, whole implementation and one result review with concrete fixes: eight principal cycles. Independent authors own large slices; one owner runs Cargo/native checks. Delta reviews are limited to the fixes, at most two. Future hypotheticals do not block acceptance, while a concrete critical defect must be fixed.

Write meaningful behavior tests before changing implementation. Run affected tests during development and the required broad package on the completed relevant result. Do not repeat unchanged green checks. No extra architecture stage, micro-packages or recurring general audits.

The authorized Windows host tests require verified rollback. Reboot/sleep/hibernation, account creation, protective ACL changes and push/merge/release require separate authorization. An unavailable mandatory check remains open while independent work continues. Finish only when the agreed real modules are usable through the UI/triggers, settings persist and application/restoration have verified results.
