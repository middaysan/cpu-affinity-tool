# Native tuning validation

The earlier canonical continuity/configuration-receipt and handle-transfer lab engine was removed during the four-stage simplification. Its commands and examples no longer exist. Historical source and reports are available in Git at `e275b2661a64676933e04eda391190c05c3c3af4`; those experiments are not acceptance evidence for the new gaming modules.

Existing ProgramData recovery evidence is inspected read-only. A journal, staging/previous artifact or an inspection error blocks native setting admission; the application does not delete, repair or recapture it. The old metadata directories/lease are not a gaming-setting change and are not automatically removed.

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

The current Windows Tweaks screen uses individual Apply/Undo actions without visible profiles or triggers. Scope and current values remain visible, with longer explanations in help. GPU selection targets a saved EXE; hardware guides and six explicit cached read-only PC checks start collapsed. Previous saved profiles/associations remain compatible and can be restored in a collapsed section. Classic affinity/priority stays in Overview.

On 2026-10-10 the five new adapters passed actual apply → read → disk reload → disable → verify original: Game Mode's stored preference, only background-capture preference, only the windowed-optimization field, GPU preference for the owned test EXE and temporary primary-screen nominal Hz. The new AC-only performance preset passed with the DC preference unchanged. Tests retained exact absence/originals before writes and used conditional independent rollback; final successful journals were empty. Display used 3840×2160×32 at120/144Hz and returned to the original144Hz. The first24Hz precondition cycle reported delayed readback during cleanup; independent fallback restored144Hz. The final display-only verifier waits up to500ms only for exact previous→desired settlement and immediately rejects a third mode or changed identity.

The final Windows suite passed478 tests with20 opt-in ignored; OS tests passed101 plus one documentation test, and the added display settlement regression passed separately. After the compact snapshot stopped polling hidden audio/PnP/USB inventories, the affected tuning batch passed115 with one ignored; final compact UI tests passed7. Shipping Clippy, formatting, release build, manifest and EXE/PDB identity passed. Lockfiles remained unchanged.

Native GUI capture confirmed the400px viewport and compact route; a separate sidecar preview correctly refused native admission because the user-local recovery belongs to another state.json source. Further GUI input was stopped by the user's physical Escape, so the complete live Apply/Undo/help/PC-check walkthrough remains open. The existing user configuration stayed byte-identical and its recovery fields/owners stayed empty. The local desktop EXE is available for user feedback; no publication occurred.

Preference read/write evidence does not establish Windows Settings UI agreement on every Windows11 build, effective GPU selection, game FPS/input latency or a benefit on every PC. The interface states stored preferences/restart requirements, missing evidence and hardware-dependent guidance; BIOS/vendor/network changes and game-load benchmark claims are not part of these probes.

Testing on the current Windows host is authorized with verified rollback. Reboot/sleep/hibernation, account creation, protective ACL changes and push/merge/release are outside this goal. Successful application of a setting proves that API path and restoration, not a universal gaming-performance benefit; compare representative A/B captures separately.
