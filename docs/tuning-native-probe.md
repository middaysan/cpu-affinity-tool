# Native tuning validation

## Compact GUI acceptance through debug MCP (2026-10-18)

The isolated optimized debug build passed actual egui input and independent
native readback on this Windows 11 host, both without elevation and in a final
elevated repeat. Seven cards made real changes: AC power preference, Game Mode,
background clips, windowed games, mouse acceleration, accessibility hotkeys and
the GPU preference for one owned test EXE. The app closed normally, reopened the
same saved state and recovery journal, retained every change/owner, then restored
each prepared baseline through its ordinary Undo button. Battery/DC and the
mouse thresholds were preserved.

The primary screen changed between supported 120/144 Hz modes at the same
resolution. Keep followed by app restart and Undo, the 15-second expiry, and
closing before confirmation followed by another app restart all passed. Each
case checked the actual native mode; the final independent conditional rollback
verified every captured original, including absent preferences. Five app launches
exited with code zero. Original user state bytes remained unchanged. Section
navigation, System help, explicit Check PC and the saved-EXE picker were also
exercised through their real widgets. This completes the managed-card and display
GUI paths left open by the earlier walkthrough; it is not an OEM/HDR/VRR or game
performance certification.

The first run exposed a debug-adapter console-code-page bug; an automated legacy
encoding regression now requires UTF-8 bytes on stdout. Separate fixture errors
(omitted stable rule IDs and an incorrect expectation that mouse thresholds
would be zeroed) were corrected without weakening production admission or mouse
preservation. Failed reports remain local alongside the successful evidence.

A separate next-logon fixture is prepared for mouse, accessibility and display.
It records real old-session baselines but finishes preparation at the exact
original Windows values. The observer requires a new same-account logon and
positive old-session absence, compares fresh values before/after retirement,
checks durable empty ownership/disabled intents and reopens the app once more.
It has not yet run at a new Windows login. Preparation and same-logon refusal
are not acceptance evidence for logout/login; the temporary task expires and
removes itself after its one observed run. No logout or reboot is automated.

## Session lifetime regression checks (2026-10-17)

Mouse, accessibility and display recovery now distinguish a Manual activation
from the lifetime of its temporary Windows session. The existing worker may
retire an expired module target and shared owners only after
[`LsaGetLogonSessionData`](https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsagetlogonsessiondata)
reports that exact old logon absent, with the same current account/desktop context
verified before and after. No old-session setting is read or written into a new
logon. Present sessions, different accounts and read failures are not expiry.
Persistent account-scoped settings retain their existing restoration behavior.

Regression fixtures cover User and display-like Device targets across saved-state
reload, startup retirement, uncertain/denied lifetime reads, subsequent Undo,
shared owners, failed recovery publication and successful unrelated Apply.
Publication failure retains both disk and in-memory originals. Mixed-field
fixtures also verify that a newly unowned no-op field does not block Undo after
an external change, while already-owned equal requests still share their original.

The explicit read-only Windows probe passed for the actual current desktop logon
(not expired) and an absent logon ID (expired). It changes no setting or session:

```powershell
cargo test --locked --manifest-path libs/os_api/Cargo.toml native_session_expiry_reads_current_and_absent_logon_without_mutation -- --ignored --test-threads=1
```

These fixtures and native reads do not constitute a new actual sign-out/reboot
or settings Apply/Undo GUI run. The remaining compact GUI smoke remains separate.

The earlier canonical continuity/configuration-receipt and handle-transfer lab engine was removed during the four-stage simplification. Its commands and examples no longer exist. Historical source and reports are available in Git at `e275b2661a64676933e04eda391190c05c3c3af4`; those experiments are not acceptance evidence for the new gaming modules.

Existing ProgramData recovery evidence is inspected read-only. A journal, staging/previous artifact or an inspection error blocks native setting admission; the application does not delete, repair or recapture it. The old metadata directories/lease are not a gaming-setting change and are not automatically removed.

## Account recovery follow-up (2026-10-13)

Persistent power and Windows preferences now identify the resource by account SID; GPU preference uses SID plus the qualified executable path. Every native operation still verifies the current caller/Explorer or exact UAC-linked identity, pins that live logon and rechecks it around reads/writes. Legacy logon-bearing recovery targets normalize through the owning adapter without recapturing the baseline. Ambiguous aliases retain the original journal and block writes. Mouse, accessibility shortcuts, display and process identities keep their original session/instance requirements; unresolved session-only evidence is not transferred to another Windows sign-in.

Regression tests cover old-logon journal reload, unchanged first baseline, equal-request sharing, Undo, foreign-value drift, another account and alias collisions. Pure native-admission tests cover the same SID with a new verified logon and reject another SID, unavailable verification and a changed exact session. A real logout/login was not performed; these tests do not prove every OEM, driver or Windows sign-in environment.

Six affected native round trips passed on this Windows host: Power Plan, Game Mode, Background clips, Windowed games, GPU preference after removing the temporary EXE, and the AC-only power preset. Each applied a real setting, read it, reloaded its isolated disk journal, disabled it and verified the original. The AC test also verified the untouched DC preference. Independent conditional rollback remained available throughout; existing user rules and application state were not replaced. These results verify stored settings and restoration, not FPS or latency gains.

Current-request feedback regressions also cover manual/autonomous retry error cleanup, partial restoration, the strongest remaining restart requirement and subsequent Apply. Successful saved-source verification removes only its preceding verification error. Confirmed withdrawal clears a rejected conflict only without field/record/pending evidence; real recovery errors and baselines remain. The disabled-platform regressions cover launch, manual Fix and monitoring: retained Windows associations cannot reserve classic affinity/priority fields when System Tweaks is disabled. The local Linux-feature run on Windows is not native Linux evidence.

## Current read-only evidence

On 2026-10-11 the user's elevated release screenshot exposed a caller/Explorer SID/session/logon admission refusal. The former exact-only comparison omitted the caller's OS-linked UAC token. The fix accepts that token only when it exactly identifies Explorer and the caller still has the same SID/session; saved user targets use the desktop logon. Other accounts/sessions, unrelated sign-ins, impersonation and changed caller/shell identities remain refused. Process identity and native setting/rollback contracts are unchanged. Regression coverage includes the linked pair, exact pair, missing/wrong link, foreign account/session and changed caller. The OS suite passed 104 tests plus one doc test on the actual host; the user's subsequent test3 screenshot confirmed elevated GUI readback on this host (Balanced AC, actual144Hz/highest144Hz, exact absent gaming preferences and existing mouse/accessibility values). This screenshot confirms admission/read publication for that launch; the next screenshot also confirmed Game Mode GUI Apply/read (On stored preference), its retained original absent preference and restart-required status. The subsequent test4 screenshot shows Game Mode returned to its original absent preference after the user-operated Undo check; this confirms the visible original stored state, not effective game performance. Permission failures use compact plain-language text while Help retains the native reason/code.

On 2026-10-08 the opt-in `gaming_native_readonly_inventory_reports_actual_display_audio_and_pnp` test passed on the current Windows desktop: 2 displays, 3 audio endpoints, 248 present devices and 1 processor group. It used real GDI, audio session and PnP APIs and made no setting changes. Counts are host observations, not hardware requirements.

Run explicitly:

```powershell
cargo test --locked --manifest-path libs/os_api/Cargo.toml gaming_native_readonly_inventory -- --ignored --nocapture
```

Pure API tests cover independent QoS/timer mask merging, hotkey-only merging, CPU Set IDs/empty originals, priority limits, exact raw interrupt values and real parent relationships. The process read characterization accepts the current instance and rejects a changed creation token or image path. These tests do not prove native apply/restore.

## Managed-module readiness

On 2026-10-11 three independent source reviews found and corrected partial Undo blocked by a foreign/unreadable sibling field, stale primary-display selection after Undo, missing-EXE GPU restoration and verbatim UNC qualification. They also identified an unverified display-baseline gap: acquisition now requires the exact original mode in the tested inventory, while existing restoration still validates only its destination. The custom/current-only driver precondition was not reproduced on hardware. Durable regressions cover retained drift/Unknown evidence across reload and publication-fault write blocking; display regressions cover individual/all Undo, no-op Apply and failed restoration retaining its exact target.

The isolated `native_roundtrip_tweak_gpu_removed_executable` passed on the actual Windows host: create one owned temporary EXE path, apply its GPU preference, remove that file, load a fresh registry and disk journal, disable and verify the original registry field and empty ownership evidence. Its independent conditional rollback remained available throughout. It did not modify any user's game EXE, display mode, power setting or saved application state. Final Windows tests passed501 with23 opt-in ignored, OS tests passed107 with one opt-in ignored plus one doc test, shipping Clippy and release EXE/PDB/manifest checks passed. One independent review of each fix found no introduced defect. This confirms the tested paths; other OEM/driver/monitor configurations and effective game performance remain unverified.

On 2026-10-08 all eight managed adapters passed actual apply → read result → disk reload → disable → verify original on the current Windows host. Power Mode AC and DC were tested separately; an additional owned-child test confirmed positive same-instance expiry and durable retirement without a dead-PID read/write. Production admission now requires verified independent saved configuration, the recovery lease and resolved native/recovery evidence.

The first accessibility run correctly failed to establish evidence because the selected hotkeys were already disabled. A test-only precondition then prepared one activation bit on a disabled accessibility feature through the same durable controller, ran the production adapter's full round trip, and restored the precondition. Final full mouse/accessibility structures matched the original; enabled features, timing, sound and unrelated flags were preserved. The preparation adapter is not registered in application code.

The eight adapters use actual SPI, power and process APIs. Process tests own hidden disposable children and kill/wait them. Every round trip has independent conditional rollback and retains its isolated evidence; a no-op, fixture or unavailable API is not accepted as native proof.

| Adapter | Required actual round trip |
| --- | --- |
| Power Plan | Select another existing scheme when available; verify and restore the original GUID. No scheme creation/deletion. |
| Power Mode | Verify supported API/source; independently preserve and restore AC/DC override GUIDs. |
| Mouse acceleration | Verify acceleration tuple and restoration while pointer speed remains unchanged. |
| Accessibility hotkeys | Preserve enabled accessibility functions, timing/other flags; verify owned shortcut fields only. |
| Process QoS | Use a disposable owned child with exact token/image; preserve timer/unknown policy bits. |
| CPU Sets | Use real topology IDs/groups and owned child; restore exact original empty/list selection. |
| Priority | Use owned child; verify original before the first relevant classic/module write and exact restoration. |
| Timer-policy experiment | Use owned child; verify only the documented process policy bit and preserve QoS. No claim of measured FPS/input-latency improvement. |

Recovery tests exercise the common mechanism with owned temporary settings: failure before write, physical write without confirmed progress, confirmed progress, shared ownership, conflicts, partial multi-field application and fresh-process disk reload. A forced child termination is allowed only with independent exact target rollback. No account/boot/VM/IPC laboratory matrix is required.

The three required forced-exit cases passed through `scripts/test-windows-tuning-recovery.ps1 -Offline`. Common tests also verify unavailable/corrupt/staged recovery, different saved-state sources, exact owner activation, publication failure, external drift/Unknown, valid empty projections, first-save initialization and guarded Session shutdown. Native callbacks do not hold GUI-facing locks.

Run actual module tests explicitly and serially; they change settings and verify rollback:

```powershell
cargo test --locked --features windows,tuning-test-controls --bin cpu-affinity-tool native_roundtrip_ -- --ignored --test-threads=1 --nocapture
```

On 2026-10-09 actual Windows GUI checks confirmed profile creation and selection, parameters saved before launch, saved-rule association, running and foreground activation, foreground release and explicit disabling through **Enabled**. The ordinary debug executable used the shipping source without test-control features. Release UAC and window launch were observed separately; automated elevated-window input was not confirmed.

The GUI test owned a new Character Map process. Direct Windows API reads checked its creation token and image on a retained handle: the applied power-policy control/state was `1/1`, and both foreground release and GUI disable restored the captured `0/0`. The card showed **Off** and **Original settings restored and verified**; saved enablement was false and the recovery document had no fields or owners. Both test processes then terminated, and a guarded comparison plus SHA256 verification confirmed exact restoration of the original application configuration. Existing user profiles and rules were preserved.

Actual Linux test/Clippy/release evidence is unavailable on this PC. On 2026-10-09 the user explicitly excluded Linux verification from this MR's acceptance. All four Windows stages are accepted with the evidence above; the local source and release EXE are ready. Linux compatibility remains unverified, and publication is outside the user's current authorization.

The module platform is disabled on Linux: no module registration, UI, actions, recovery or process/foreground observation. Ordinary rule/state saves retain the exact raw tuning data without updating associations. Linux module-native round trips are outside the current scope; the existing Linux beta build and regression contract remains separate from Windows module readiness.

The disable-path regression run on 2026-10-09 passed four initially failing tests for the empty registry, unavailable route, refused module requests/no idle polling and exact raw-data retention during ordinary rule changes. A common worker test also confirms no recovery-source access, observations or writes during saves/shutdown. Windows tests passed (453, one opt-in ignored); the Linux-feature test binary on the Windows host passed (437, one ignored), and both feature paths passed shipping Clippy. This verifies the disabled composition path, not the actual Linux backend or native Linux build.

The current System Tweaks screen uses individual Apply/Undo actions without visible profiles or triggers. Scope and current values remain visible, with longer explanations in help. A visible segmented switch selects System, GPU / network, Check PC or Restore. GPU selection targets a saved EXE; the six cached read-only PC checks require an explicit request. Previous saved profiles/associations remain compatible and can be restored in Restore. Classic affinity/priority stays in Affinity groups.

On 2026-10-10 the five new adapters passed actual apply → read → disk reload → disable → verify original: Game Mode's stored preference, only background-capture preference, only the windowed-optimization field, GPU preference for the owned test EXE and temporary primary-screen nominal Hz. The new AC-only performance preset passed with the DC preference unchanged. Tests retained exact absence/originals before writes and used conditional independent rollback; final successful journals were empty. Display used 3840×2160×32 at120/144Hz and returned to the original144Hz. The first24Hz precondition cycle reported delayed readback during cleanup; independent fallback restored144Hz. The final display-only verifier waits up to500ms only for exact previous→desired settlement and immediately rejects a third mode or changed identity.

The final Windows suite passed478 tests with20 opt-in ignored; OS tests passed101 plus one documentation test, and the added display settlement regression passed separately. After the compact snapshot stopped polling hidden audio/PnP/USB inventories, the affected tuning batch passed115 with one ignored; final compact UI tests passed7. Shipping Clippy, formatting, release build, manifest and EXE/PDB identity passed. Lockfiles remained unchanged.

Native GUI capture confirmed the400px viewport and compact route; a separate sidecar preview correctly refused native admission because the user-local recovery belongs to another state.json source. Further GUI input was stopped by the user's physical Escape, so the complete live Apply/Undo/help/PC-check walkthrough remains open. The existing user configuration stayed byte-identical and its recovery fields/owners stayed empty. The local desktop EXE is available for user feedback; no publication occurred.

Preference read/write evidence does not establish Windows Settings UI agreement on every Windows11 build, effective GPU selection, game FPS/input latency or a benefit on every PC. The interface states stored preferences/restart requirements, missing evidence and hardware-dependent guidance; BIOS/vendor/network changes and game-load benchmark claims are not part of these probes.

Testing on the current Windows host is authorized with verified rollback. Reboot/sleep/hibernation, account creation, protective ACL changes and push/merge/release are outside this goal. Successful application of a setting proves that API path and restoration, not a universal gaming-performance benefit; compare representative A/B captures separately.

## Retired AC CPU controls (2026-10-15)

The user subsequently withdrew frequency, high-clock and parking controls because actual enforcement depends on the platform. New power-tweak applies now change only the AC power-mode preference. CPU editor controls and new configured CPU requests are unavailable. The following evidence is historical and supports compatibility restoration: old configurations, exact bindings and native Undo coverage remain intact so previously applied settings can be restored after restart.

The existing power-mode tweak now stores a requested frequency ceiling in 100 MHz steps (zero means no cap), an optional high-clock request, and exact base-plan/Performance-overlay AC bindings. Maximum performance 100%, performance-oriented EPP, and disabled-to-enabled boost are requested only where values differ; already-enabled boost is preserved. High clocks additionally request minimum performance 100%. Battery/DC, parking and idle policies are not written. Original nullable DWORDs and module bindings use the existing controller and recovery document, without a second power-plan lifecycle.

Actual Windows 11 validation applied 4500 MHz twice, with high clocks off and on. Each cycle checked native stored values, hydrated a fresh controller from disk, disabled the saved intent and verified all original values, including absent overrides. The existing custom base plan remained selected, the DC mode remained unchanged and both parking-class indices remained unchanged. These checks prove stored policy and exact restoration on this host; they do not prove that firmware honors a ceiling, measured fixed clocks, reduced fan noise or more FPS.

The first native cycle exposed ERROR_ACCESS_DENIED when deleting a newly created override through ordinary registry access. Its baseline journal was retained and subsequently restored exactly before repeating both successful cycles. The correction checks the caller's existing SeRestorePrivilege before creating absent overrides, restores its prior token state and reopens the exact retained key with an empty subkey for deletion of ACSettingIndex only. Restoration changes no ACL, account or other value and does not delete setting containers. Recovery also repeats guarded runtime refresh when bytes already match, retaining evidence if refresh fails. See the [Windows registry API contract](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regcreatekeyexw).

Opt-in evidence: native_roundtrip_tweak_cpu_power_options; explicit retained-journal recovery: native_roundtrip_tweak_cpu_power_00_restore_retained. Local logs: target/cpu-power-native-recovery.log and target/cpu-power-native-recovery.stderr.log. Remaining user smoke: open Settings at 400px, choose a limit, Apply, restart the app, Undo; compare actual frequencies/temperature under the same game load separately.

## Compact hybrid and parking controls (2026-10-15 follow-up)

User feedback replaces the large logarithmic range with linear1000–7000MHz sliders in100MHz steps and explicitly authorizes parking controls. Two verified CPU classes expose separate limits: MaxFrequency1 for the higher-performance P class and MaxFrequency for the lower-performance E class. Observation caches the existing CPU Set inventory on the worker; rendering does not query Windows. Fresh preparation rejects separate limits without verified two-class support. A ceiling above a core type's hardware maximum does not overclock it.

New controls request minimum performance100% with high clocks checked, or0% when unchecked. Allow core parking checked requests parking minimum0% and maximum100%; unchecked requests both100% for both classes. Idle-state and battery/DC policies are unchanged. Existing shared ceilings and omitted parking/minimum controls retain their previous behavior during recovery. Old saved values outside the new UI range stay valid and are clamped only in a new editor draft, never in the retained baseline.

The actual native_roundtrip_tweak_cpu_power_hybrid_parking test passed on this Windows host with two reported CPU classes: P4500MHz/E2500MHz, first high clocks off/parking allowed, then high clocks on/parking forbidden. Both cycles changed14 fields, reloaded their disk journals, disabled and exactly restored all originals; both journals finished empty. The original active plan and DC preference remained unchanged. These checks prove stored policies and exact Undo, not measured core frequencies, energy use, game FPS or every firmware implementation. Logs: target/cpu-power-hybrid-native.log and .stderr.log. Windows547 tests, OS125+doc1 passed; closest headless coverage renders both sliders/checkboxes at compact width. Physical clock/heat and full GUI feedback remain user-operated.

Microsoft references: [MaxFrequency](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/options-for-perf-state-engine-maxfrequency), [core parking minimum](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/options-for-core-parking-cpmincores), [heterogeneous power parameters](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/configure-processor-power-management-options).

### Actual clock investigation (2026-10-15)

On this i5-14600KF host, stored P4000/E3000 MHz ceilings are **not enforced**. A one-off diagnostic used the existing typed native APIs, saved exact originals to disk before writes and applied bounded changes with conditional restoration. The documented effective-power-mode V2 callback reported HighPerformance (3), not GameMode (5), throughout. An explicit Game Mode Off preference did not change the result.

| Temporary AC policy | P estimate | E estimate |
| --- | ---: | ---: |
| Game Mode off; original minimum | 5290 MHz | 3993 MHz |
| Game Mode off; minimum 0% in base and Performance overlay | 5289 MHz | 3993 MHz |
| Same; maximum performance 50% in both | 5290 MHz | 3993 MHz |

Estimates use per-logical-processor Windows performance feedback counters under a six-second arithmetic load and agree with the user's CPU-Z observations. They are separate from stored-policy readback. Kernel-Processor-Power event55 for all20 logical processors reports Performance state type: None (PerformanceImplementation0), with minimum/maximum performance100%. This indicates no performance-state control interface was exposed to Windows at boot; the specific BIOS or system configuration responsible is not yet established. Game Mode is therefore not required to reproduce this host's failure, even though its separate PPM profile can affect other systems.

The test restored and verified all12 captured CPU fields, the original absent Game Mode preference, active base scheme and configured AC/DC power modes. An independent registry read found zero CPU restoration mismatches, including exact absent overrides. Frequency ceilings, battery/DC and parking policy were not changed by this diagnostic. Original application-owned settings remain applied. Local evidence: target/cpu-clock-native{,-baseline,-result}.json/log files and cpu-clock-native.exit0; no shipping diagnostic entrypoint was added. BIOS changes/reboot and an application fix remain outside this investigation.

Microsoft documents the [effective mode notification](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/nf-powersetting-powerregisterforeffectivepowermodenotifications) and [mode enumeration](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/ne-powersetting-effective_power_mode).

Follow-up read-only checks identified a MSI PRO B760-P WIFI DDR4/MS-7D98 with BIOS1.E0. Both Intel power drivers are running, processor devices have no PnP errors, and the CPU driver signature is valid. CPUID6EAX0x00880077 does not advertise HWP/Speed Shift or HWP EPP, although [Intel's i5-14600KF specifications](https://www.intel.com/content/www/us/en/products/sku/236778/intel-core-i5-processor-14600kf-24m-cache-up-to-5-30-ghz/specifications.html) list Speed Shift support. EIST hardware support is advertised, but this does not prove its firmware switch is enabled. No active hypervisor, disabled processor-driver override, or obvious current BCD CPU restriction was found. Firmware-disabled/masked performance control is the leading hypothesis; actual BIOS settings still require inspection. The [MSI Intel700 BIOS manual](https://download.msi.com/archive/mnu_exe/mb/Intel700BIOS.pdf) documents Intel Speed Shift Technology, EIST and Fixed/Dynamic CPU Ratio Mode. No BIOS, boot, driver or policy settings were changed during these follow-up checks.
