# Windows 11 gaming modules: evidence and reversible scope

Research date: 2026-10-07 (Europe/Kiev). Companion to [the modular platform proposal](modular-gaming-features-plan.md), reviewed at PR #22 head `b7a3666`. These are recommendations for the implementation backlog, not shipped features or measured performance results.

## Recommendation

Keep F1-F4 and their two-fake-module acceptance gate. Build a small catalog of observable, precisely reversible settings; add read-only measurement and diagnosis early. A setting with a documented API can be a manageable product feature without being an FPS improvement on every PC. Reversibility, supported mechanism, useful effect, and measured benefit are separate admission requirements.

Recommended initial catalog:

1. `measurements`: bounded benchmark CSV import and A/B comparisons first.
2. `display_audit` / `latency_advisor`: current display/refresh and available native game/driver latency controls, with guidance.
3. `power_plan`: switch an existing plan with exact GUID restoration and verified caller context.
4. `power_mode`: separate Windows 11 user-configured AC/DC power modes, with requested/effective state distinction.
5. `process_qos`: explicit verified-process execution-speed policy, including selected background work; no automatic system-wide sweep.
6. `cpu_sets`: measured process-placement experiments after the existing hard-affinity writer is integrated.
7. `process_priority`: modest session policy after baseline capture and a single-writer migration.
8. `audio_diagnostics` and `irq_diagnostics` / `usb_topology`: on-demand observation of the actual layer involved.
9. Optional `mouse_acceleration` and `accessibility_hotkeys`: input conveniences with user/accessibility/foreground gates, explicitly not FPS boosts.
10. Native Settings cards for Game Mode, background recording, windowed optimizations, and preferred GPU; managed adapters only after a supported mechanism and native restoration matrix exist.

Add `process_timer_policy` to the measured experimental backlog. Driver latency/frame-cap controls are worthwhile later adapters, initially available through native guidance. Defer automatic audio-engine pinning, IRQ/MSI mutation, parking overrides, HAGS, and display-mode switching from the initial safe-toggle catalog.

All mutating modules default to unmanaged. Turning management off restores the captured baseline when comparison and verification allow it; it does not set a guessed Windows default. An unavailable read/restore path makes the module unavailable for mutation.

## What current sources support

### Power Mode is a useful missing module

Windows 11 has documented `PowerGet/SetUserConfiguredACPowerMode` and DC counterparts. They manage the user's separate adapter/battery choices among efficiency, balanced, and performance. The requested mode is a vote and may differ from effective mode. This is distinct from choosing a classic scheme. Probe actual exports/results, capture the relevant original GUIDs, and show power-source transitions. A performance preference can cost power/thermal headroom; it is not a guarantee of a sustained game gain. [AC setter](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powersetuserconfiguredacpowermode), [AC getter](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powergetuserconfiguredacpowermode), [DC setter](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powersetuserconfigureddcpowermode), [DC getter](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powergetuserconfigureddcpowermode).

Power Plan also needs a context correction: Microsoft describes `PowerSetActiveScheme` as operating for the current user. Keep conservative machine-wide resource coordination because power behavior is shared, but verify caller SID/logon context and native multi-user behavior rather than assuming a context-free machine switch. Alternate-account UAC must fail closed when the intended user cannot be proved. Coordinate plan and mode resources so one cannot silently invalidate the other's observation. [PowerSetActiveScheme](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/nf-powersetting-powersetactivescheme).

### GPU/display/game settings matter more than a universal registry preset

Current Windows guidance supports optimizations for compatible windowed/borderless DX10/11 games, preferred GPU selection, and game restart after changes. Auto HDR interacts with the windowed-optimization switch. Native UI support does not establish a stable registry read/write contract for this utility; use guidance first and preserve combined preference fields in any later tested adapter. [Microsoft windowed-game guidance](https://support.microsoft.com/en-us/windows/hardware/display-graphics/optimizations-for-windowed-games-in-windows-11).

Audit actual refresh rate before obscure changes. Microsoft documents that DRR can limit the maximum refresh of games not designed for VRR; a conditional native-settings recommendation is useful. Do not force maximum refresh on battery or change resolution silently. Mode mutation needs independent confirmation/recovery and is deferred. [Microsoft refresh/DRR guidance](https://support.microsoft.com/en-us/windows/hardware/display-graphics/change-the-refresh-rate-on-your-monitor-in-windows).

A latency advisor should explain game-integrated Reflex/Anti-Lag 2, supported driver queue controls, frame limits, VRR/V-Sync, and the GPU-bound/tearing tradeoff. These choices can reduce queues while lowering reported average FPS; the highest displayed FPS is not necessarily the lowest input delay. Avoid a fixed global cap or additive "all optimizations" preset. NVIDIA's 2020 guide remains useful for mechanisms, but its blanket exclusive-fullscreen preference is not a current universal Windows 11 rule. Advanced IRQ/MSI/power tweaks are explicitly situational even in that guide. [NVIDIA latency guide](https://www.nvidia.com/en-gb/geforce/guides/system-latency-optimization-guide/).

Reflex and Anti-Lag 2 require application integration; this app cannot truthfully enable them in arbitrary games. An ADLX control can be GPU-wide and may briefly blank attached displays; it must not be advertised as a per-EXE control without proof. Expose vendor settings individually with capability and restore checks. [AMD Anti-Lag](https://www.amd.com/en/products/software/adrenalin/radeon-software-anti-lag.html), [ADLX 3D interfaces](https://gpuopen.com/manuals/adlx/adlx-sdk-references/adlx-interfaces/3d-graphics/).

A current native-guidance opportunity is Xbox mode/full-screen experience on supported Windows 11 versions/markets. It changes shell/app behavior; background-process restrictions are documented for handheld startup mode. Offer supported native guidance, never replace Explorer or terminate apps to imitate it. [Microsoft Xbox mode](https://support.microsoft.com/en-us/servicing/os/windows/docs/2025/10/windows-gaming-full-screen-experience).

### Process placement is conditional

Keep process QoS, CPU Sets, and modest priority controls as explicit choices. Do not ship "all games P-cores only", "disable SMT", "never CPU 0", or a fixed audio core. Intel's current threading guidance warns against assuming P-only or hard affinity fixes performance and recommends benchmarking topology/workload assumptions. Preserve the current scheduling behavior until a separately tested ownership migration exists. [Intel threading guidance](https://www.intel.com/content/www/us/en/developer/articles/technical/optimizing-threading-for-gaming-performance.html), [Microsoft CPU Sets](https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets).

## Timer resolution and interrupt timers

Timer resolution, QPC timestamp precision, periodic kernel clock interrupts, device ISR/DPC placement, and mouse report rate are different mechanisms. Do not label them with one "interrupt timer" checkbox.

Since Windows 10 2004, a `timeBeginPeriod` call does not guarantee higher resolution for another process that made no request. Windows 11 may ignore a window-owning process's request when it is fully occluded, minimized or otherwise non-visible, and non-audible. Higher timer resolution can also increase scheduling/power cost. Therefore a successful call in our tray process is not proof that a selected game got a finer timer or lower input latency. [Microsoft timeBeginPeriod](https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod), [visibility/audibility policy](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation).

`process_timer_policy` is a narrower optional experiment: explicitly disable `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION` for a verified process so its existing requests can be honored. It creates no new 0.5/1 ms request. Benefit is unproven for an ordinary visible game and impossible to infer from the policy bit alone. [Microsoft process timer policy](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation).

This policy and execution-speed QoS share the `ProcessPowerThrottling` structure. Require a successful capability read, snapshot both masks, and merge only owned baseline bits with current unrelated bits during restore. Preserve the original system-managed state; never replay a stale whole structure or clear every bit. Compare/verify both modules together. A reused PID or ended instance is never touched. Getter support must be tested on each supported build. [GetProcessInformation](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocessinformation).

Keep HPET/BCD `useplatformclock`, `useplatformtick`, and `disabledynamictick` excluded: Microsoft documents the boot options as debugging controls. Global/undocumented `NtSetTimerResolution` and `GlobalTimerResolutionRequests` workarounds do not have a verified safe arbitrary-game contract in the checked primary sources. A kernel timer-resolution driver is outside this product's scope. This is an evidence/support decision, not a claim that no individual machine can ever show an empirical change. [BCDEdit options](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/bcdedit--set), [kernel ExSetTimerResolution](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-exsettimerresolution).

## Audio placement and IRQ/DPC

"Move the sound-card process" can mean several different operations:

| Layer | Actual target | Decision |
| --- | --- | --- |
| Game/voice processing | Application's user-mode threads | Existing verified-process scheduling controls, only with a measured need. |
| Shared engine/APOs | `audiodg.exe` | Diagnose first; placement can affect other audio clients and can be access-denied. |
| Device interrupts | HDA/USB/HDMI/GPU device or parent controller | Read-only inventory first; process affinity does not move these interrupts. |
| Driver deferred work | Driver-owned DPCs | Measure actual placement; driver policy can choose a separate DPC target. |
| Multimedia scheduling | Participating audio/application threads | Do not rewrite global MMCSS settings to imitate app/driver participation. |

The shared engine and driver/application buffer behavior are separate. A third-party process policy cannot make a game request low-latency audio buffers. Audio session PID information identifies a client and can represent a multi-process session; it is not a reliable endpoint-to-audiodg mapping. Protected processes can reject modification rights, which must be treated as unavailable without a bypass. [Audio components](https://learn.microsoft.com/en-us/windows/win32/coreaudio/user-mode-audio-components), [low-latency audio](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio), [session PID contract](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getprocessid), [process access rights](https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights).

Add bounded `audio_diagnostics`: endpoint/session inventory, reliable device/driver/transport identification, readable scheduling state, native/vendor buffer/effects guidance, and imported trace analysis. A later owned ETW collector requires separate lifecycle/overhead/privacy validation. Never record audio content for this diagnosis.

Add `irq_diagnostics` / `usb_topology`: real PnP/controller relationships, processor groups, stored interrupt policy, and trace-observed ISR/DPC behavior. A USB controller can serve mouse, audio, and other devices. Device affinity is group-relative; a driver can separately target DPCs. Do not invent a per-sound-card CPU from a global index. [Interrupt affinity](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/interrupt-affinity-and-priority), [DPC targeting](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-kesettargetprocessordpcex), [USBView](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/usbview-sample-application/).

Defer `audio_process_placement` until a reproducible trace demonstrates contention and a verified writable instance has an exact baseline. No automatic audiodg CPU 2/High/Realtime preset, no audio-service restart, and no global enhancement/sample-rate change. IRQ/MSI mutation also stays outside the initial session-toggle set: restart, device/boot failure, and recovery-before-app-access require additional product support. [MSI driver configuration](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/enabling-message-signaled-interrupts-in-the-registry).

## Disposition of every original module

This table is a product recommendation. "Managed candidate" still requires F1-F4, TDD, native capability/restoration evidence, and all defaults off.

| Original module | Recommended disposition | Required boundary |
| --- | --- | --- |
| `power_plan` | First managed candidate | Existing scheme only; exact GUID; caller context; conservative shared ownership; no plan fabrication. |
| `mouse_acceleration` | Optional convenience | Exact tuple; verified interactive logon; foreground eligibility; restore on focus loss; raw-input games may be unaffected. |
| `accessibility_hotkeys` | Optional convenience | Sticky/Filter/Toggle hotkey flags only; skip an already-enabled accessibility feature; foreground restore; no feature disablement. |
| `process_qos` | Managed candidate | Explicit targets; original control/state bits; native getter/setter validation; no blanket background-process policy. |
| `cpu_sets` | Optional measured scheduling control | Real topology; exact set-list baseline; one scheduling writer; no P-only/SMT/CPU-index universal preset. |
| `process_priority` | Optional modest session control | Capture before existing launch/monitor writes; one writer; no High/Realtime gaming default. |
| `game_mode` | Native guidance first | Correct build/user; no deprecated API or assumed universal registry recipe. |
| `background_capture` | Native guidance first | Background recording only; preserve Game Bar and other capture functions. |
| `windowed_optimizations` | Native guidance first | Compatible game/API/build; preserve Auto HDR and per-app exceptions; game restart. |
| `gpu_preference` | Native guidance first | Multi-GPU relevance; preserve whole existing override; distinguish requested GPU from observed renderer. |
| `core_parking` | Defer from initial catalog | Clone-only experiment; ownership/cleanup; AC/DC/hybrid behavior; measured need. |
| `hags` | Native guidance/manual A/B only initially | Supported driver/build; apply and restore require restart/effective-state tracking; no preferred universal value. |
| `irq_affinity` | Promote read-only inventory; defer mutation | Real device/group mapping; shared-controller scope; restart and recovery independent of a running app. |
| `nvidia_profile` | Later focused vendor adapter | One documented setting at a time; restore individual override/absence; read inherited/effective scope; no whole-profile reset. |
| `amd_3d` | Later focused vendor adapter | Actual ADLX capability and GPU/app scope; field restore; display interruption and multi-game contention. |
| `measurements` | Promote early; CSV import first | Explicit schema/metric validity, missing values, collector overhead and comparable workloads. |
| `display_profile` | Split off early read-only `display_audit`; defer mode mutation | Valid topology/modes; no silent resolution change; independent confirmation and recovery. |

Input safeguards come from Microsoft's fullscreen/accessibility guidance; raw mouse input bypasses Windows pointer ballistics. The foreground restriction for mouse changes is an additional conservative product rule. [Accessibility shortcut guidance](https://learn.microsoft.com/en-us/windows/win32/dxtecharts/disabling-shortcut-keys-in-games), [raw mouse input](https://learn.microsoft.com/en-us/windows/win32/dxtecharts/taking-advantage-of-high-dpi-mouse-movement).

## Safety requirements to add to the foundation

1. Extend user/session identity with a verified logon incarnation (for example a logon LUID) rather than relying on a reusable session number. A persistent user record and a session record need different lifetime/recovery rules.
2. Session input settings need authoritative foreground eligibility and focus-loss release, not only game-process lifetime. Unknown focus/context releases or blocks the request.
3. Preserve exact original absence/type/bytes/GUID/list and both controlled/uncontrolled bits. Compare only owned fields; keep unrelated external changes intact.
4. Power Plan/Mode and QoS/timer policy need shared-structure coordination tests. Native successful writes are not proof of performance benefit or effective state.
5. Refuse every write if the baseline, target, lease, journal, or required restore adapter is unavailable. Keep pending resources blocked when compensation is unresolved.
6. Keep the current no-service design honest: after forced termination global/user changes may remain until the next recovery launch. Do not label such a module "instant restore on crash". Immediate independent rollback would require additional architecture and testing.
7. Preserve separate pending-restart state on both apply and restore. A restart-dependent tweak is not a temporary game-session toggle.
8. No mutation of protected processes, security/boot/firmware settings, unknown driver values, or unverified elevated-user context.
9. Test disable while applying, lost focus, rule deletion, same-value/different-value games, two instances, external drift, cancelled/failed restore, ended/reused PID, logoff/new logon, and every journal interruption boundary.
10. Restore means the original state verified on the owned resource; a green checkbox or a successful API return is insufficient.

## Measurement before claiming benefit

The initial `measurements` implementation should import user-provided PresentMon/benchmark CSVs, avoiding a new executable dependency. Record source/version, process/swap chain, presentation mode, render/generated frame distinction, capture method and supported metric availability. Current PresentMon exposes optional input/PC/display latency fields; generic frame time alone does not certify hardware click-to-photon latency. [PresentMon console metrics](https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md).

Use multiple warmed-up A/B or ABBA runs with one change, the same scene/build/settings, similar power/thermal conditions and a fixed duration. Report frame-time percentiles, variance and stutters, plus credible latency/audio measures where available. Define "1% low" calculations instead of mixing incompatible formulas. A collector can alter the workload; test its overhead and display unavailable values as unavailable.

Audio/IRQ experiments need trace correlation with actual glitches or stutter intervals, including CPU ready time and DPC/ISR durations by module/CPU. A single worst DPC value is not a proof of the game's input delay. Foreground input-event interarrival tests measure application delivery, not certified firmware report rate. [WPA CPU/DPC analysis](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/cpu-analysis).

Promote a hardware-specific experimental recommendation only after exact restoration and a repeatable useful effect are both shown. This research ran no game benchmark or audio/driver experiment and supplies no percentage FPS/latency claim.

## Excluded presets and why

- HPET/BCD/global timer forcing: boot/system scope and no verified general game benefit.
- Global MMCSS `SystemResponsiveness=0`, unused `GPU Priority`, and scheduler-quantum recipes: no supported universal game interpretation; multimedia APIs operate on participating threads. [MMCSS contract](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service).
- Blanket USB power-saving disable: wider scope and contrary to Microsoft's general recommendation. [USB selective suspend](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/usb-selective-suspend).
- MSI/IRQ overrides on every device, audiodg High/Realtime/core-number presets, forced driver/controller restart: hardware/shared-workload and recovery risks.
- Security/VBS/HVCI/Defender/UAC changes, service/package stripping, Windows Update disable, RAM/standby/pagefile cleaners, shader-cache deletion as a performance routine, and blanket TCP/Nagle/offload changes: outside the proposed low-impact catalog; no admission based on a tweak-list anecdote.
- BIOS/firmware/overclock/undervolt/C-state changes and third-party input/driver injection: outside this app's supported boundary.
- Universal HAGS/MPO/fullscreen/VRR/V-Sync values: hardware/game/driver-specific; native guidance or issue-specific measurement only.

## Delivery adjustment

The source proposal's package IDs remain proposal IDs, not canonical local stages. Map accepted implementation work into `.codex/ROADMAP.md` without replacing historical stages.

1. F1-F4 remain the foundation sequence. Include the context/foreground/shared-structure/recovery safeguards above in their acceptance tests.
2. At the integration gate, add read-only measurement import, display/native latency guidance, and basic audio/device inventory early.
3. Keep I1 as the first production mutation: existing Power Plan. Add Power Mode as a separate subsequent slice with its own AC/DC/context smoke.
4. Add input conveniences and process QoS/CPU Sets/priority as separate opt-in modules, with measured scheduling recommendations rather than presets.
5. Test `process_timer_policy` in a native requesting/non-requesting and visible/occluded harness before exposing it as an experiment. It never becomes a global timer-resolution toggle.
6. Add focused vendor settings only after scope/restore proof. Keep reboot/device/boot-sensitive mutations outside the initial catalog.

Native smoke records Windows build, caller/interactive logon, hardware/driver, initial/applied/restored fields, external drift, power/focus changes and failure recovery. This document does not close any existing manual gate or authorize Windows settings changes on this PC.
